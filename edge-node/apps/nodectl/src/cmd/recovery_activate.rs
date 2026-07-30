use std::{
    collections::VecDeque,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use iotkit_core_publish::mqtt::MqttBinding;
use iotkit_core_recovery::{
    RecoveryActivationResult, RecoveryCompletion, RecoveryCompletionAck, RecoveryError,
    RecoveryStartupMode, apply_recovery_activation, complete_recovery_activation, startup_mode,
};
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, Outgoing, QoS, Transport};

#[derive(clap::Args)]
pub struct ActivateArgs {
    #[arg(long = "candidate-db")]
    pub candidate_db: PathBuf,
    #[arg(long = "broker-host")]
    pub broker_host: String,
    #[arg(long = "broker-port", default_value_t = 8883)]
    pub broker_port: u16,
    #[arg(long = "password-file")]
    pub password_file: PathBuf,
    #[arg(long = "ca-file")]
    pub ca_file: PathBuf,
    #[arg(long = "timeout-seconds", default_value_t = 300)]
    pub timeout_seconds: u64,
}

pub fn activate(args: ActivateArgs) -> Result<(), RecoveryError> {
    if args.broker_host.is_empty() || args.timeout_seconds == 0 {
        return Err(RecoveryError::InvalidConfiguration);
    }
    let password = read_owner_only(&args.password_file)?;
    let ca = read_regular(&args.ca_file)?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| RecoveryError::Storage)?
        .block_on(activate_async(args, password, ca))
}

async fn activate_async(
    args: ActivateArgs,
    password: String,
    ca: Vec<u8>,
) -> Result<(), RecoveryError> {
    let conn =
        rusqlite::Connection::open(&args.candidate_db).map_err(|_| RecoveryError::Storage)?;
    if !activation_mode_allowed(&startup_mode(&conn)?) {
        return Err(RecoveryError::RecoveryConflict);
    }
    let identity = iotkit_core_ledger::load_edge_node_identity(&conn)
        .map_err(|_| RecoveryError::RecoveryConflict)?;
    let binding = MqttBinding::for_edge_node(&identity.edge_node_id)
        .map_err(|_| RecoveryError::InvalidConfiguration)?;
    install_crypto_provider()?;
    let mut options = MqttOptions::new(&binding.client_id, &args.broker_host, args.broker_port);
    options.set_keep_alive(Duration::from_secs(15));
    options.set_clean_session(false);
    options.set_credentials(&binding.username, password);
    options.set_transport(Transport::tls(ca, None, None));
    let (client, mut event_loop) = AsyncClient::new(options, 8);
    client
        .subscribe(&binding.recovery_request_topic, QoS::AtLeastOnce)
        .await
        .map_err(|_| RecoveryError::RecoveryConflict)?;
    client
        .subscribe(&binding.recovery_completion_topic, QoS::AtLeastOnce)
        .await
        .map_err(|_| RecoveryError::RecoveryConflict)?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(args.timeout_seconds);
    let mut publish_receipts = PublishReceiptTracker::default();
    loop {
        let event = tokio::time::timeout_at(deadline, event_loop.poll())
            .await
            .map_err(|_| RecoveryError::RecoveryConflict)?
            .map_err(|_| RecoveryError::RecoveryConflict)?;
        match event {
            Event::Incoming(Incoming::Publish(publication))
                if publication.topic == binding.recovery_request_topic =>
            {
                let request =
                    iotkit_core_recovery::RecoveryActivationRequest::decode(&publication.payload)
                        .map_err(|_| RecoveryError::RecoveryControlInvalid)?;
                if request.edge_node_id != identity.edge_node_id {
                    return Err(RecoveryError::RecoveryConflict);
                }
                let result = apply_recovery_activation(&conn, &request, now_ms())?;
                publish_result(&client, &binding, &result).await?;
                publish_receipts.enqueued(LocalPublishKind::Result);
            }
            Event::Incoming(Incoming::Publish(publication))
                if publication.topic == binding.recovery_completion_topic =>
            {
                let completion = RecoveryCompletion::decode(&publication.payload)
                    .map_err(|_| RecoveryError::RecoveryControlInvalid)?;
                complete_recovery_activation(&conn, &completion, now_ms())?;
                let acknowledgement = RecoveryCompletionAck::for_completion(&completion, now_ms())
                    .map_err(|_| RecoveryError::RecoveryControlInvalid)?;
                publish_completion_ack(&client, &binding, &acknowledgement).await?;
                publish_receipts.enqueued(LocalPublishKind::CompletionAck);
            }
            Event::Outgoing(Outgoing::Publish(packet_id)) => {
                publish_receipts.outgoing(packet_id)?;
            }
            Event::Incoming(Incoming::PubAck(ack)) if publish_receipts.acknowledged(ack.pkid) => {
                println!(
                    "{}",
                    serde_json::json!({
                        "status": "recovered",
                        "edge_node_id": identity.edge_node_id,
                        "ledger_epoch": iotkit_core_ledger::ledger_epoch(&conn)
                            .map_err(|_| RecoveryError::RecoveryConflict)?,
                    })
                );
                return Ok(());
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LocalPublishKind {
    Result,
    CompletionAck,
}

#[derive(Default)]
struct PublishReceiptTracker {
    enqueued: VecDeque<LocalPublishKind>,
    completion_ack_packet_id: Option<u16>,
}

impl PublishReceiptTracker {
    fn enqueued(&mut self, kind: LocalPublishKind) {
        self.enqueued.push_back(kind);
    }

    fn outgoing(&mut self, packet_id: u16) -> Result<(), RecoveryError> {
        match self.enqueued.pop_front() {
            Some(LocalPublishKind::Result) => Ok(()),
            Some(LocalPublishKind::CompletionAck) => {
                self.completion_ack_packet_id.get_or_insert(packet_id);
                Ok(())
            }
            None => Err(RecoveryError::RecoveryConflict),
        }
    }

    fn acknowledged(&self, packet_id: u16) -> bool {
        self.completion_ack_packet_id == Some(packet_id)
    }
}

fn activation_mode_allowed(mode: &RecoveryStartupMode) -> bool {
    matches!(
        mode,
        RecoveryStartupMode::FencedCandidate { .. }
            | RecoveryStartupMode::AwaitingCompletion { .. }
            | RecoveryStartupMode::Recovered { .. }
    )
}

async fn publish_completion_ack(
    client: &AsyncClient,
    binding: &MqttBinding,
    acknowledgement: &RecoveryCompletionAck,
) -> Result<(), RecoveryError> {
    client
        .publish(
            &binding.recovery_completion_ack_topic,
            QoS::AtLeastOnce,
            false,
            serde_json::to_vec(acknowledgement).map_err(|_| RecoveryError::RecoveryConflict)?,
        )
        .await
        .map_err(|_| RecoveryError::RecoveryConflict)
}

async fn publish_result(
    client: &AsyncClient,
    binding: &MqttBinding,
    result: &RecoveryActivationResult,
) -> Result<(), RecoveryError> {
    client
        .publish(
            &binding.recovery_result_topic,
            QoS::AtLeastOnce,
            false,
            serde_json::to_vec(result).map_err(|_| RecoveryError::RecoveryConflict)?,
        )
        .await
        .map_err(|_| RecoveryError::RecoveryConflict)
}

fn read_owner_only(path: &Path) -> Result<String, RecoveryError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| RecoveryError::InvalidConfiguration)?;
    if !metadata.file_type().is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(RecoveryError::InvalidConfiguration);
    }
    let value = fs::read_to_string(path).map_err(|_| RecoveryError::Storage)?;
    let value = value.trim_end_matches(['\r', '\n']).to_owned();
    if value.is_empty() {
        return Err(RecoveryError::InvalidConfiguration);
    }
    Ok(value)
}

fn read_regular(path: &Path) -> Result<Vec<u8>, RecoveryError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| RecoveryError::InvalidConfiguration)?;
    if !metadata.file_type().is_file() {
        return Err(RecoveryError::InvalidConfiguration);
    }
    let value = fs::read(path).map_err(|_| RecoveryError::Storage)?;
    if value.is_empty() {
        return Err(RecoveryError::InvalidConfiguration);
    }
    Ok(value)
}

fn install_crypto_provider() -> Result<(), RecoveryError> {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    rustls::crypto::CryptoProvider::get_default()
        .is_some()
        .then_some(())
        .ok_or(RecoveryError::InvalidConfiguration)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

#[cfg(test)]
#[path = "../../tests/unit/cmd/recovery_activate_tests.rs"]
mod tests;
