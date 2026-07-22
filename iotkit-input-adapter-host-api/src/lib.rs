//! Stable, supervision-free northbound composition boundary for IoTKit input adapters.

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use iotkit_ingest_client::IngestClient;
pub use iotkit_ingest_client::{
    AbandonReason, DeliveryOutcome, DeliveryReceipt, EnqueuedEnvelope, QueueSubmitError,
    RetryHandle,
};
use iotkit_ingest_contract::ReadingItem;
use tokio::sync::{mpsc, oneshot, watch};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifierError {
    kind: &'static str,
    value: String,
}

impl fmt::Display for IdentifierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid {} {:?}", self.kind, self.value)
    }
}

impl std::error::Error for IdentifierError {}

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

string_id!(AdapterTypeId);
string_id!(AdapterInstanceId);
string_id!(ConfiguredSource);

#[derive(Debug, Clone, PartialEq)]
pub enum AdapterConfigScalar {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
}

impl AdapterTypeId {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        if valid_segmented_lower_id(&value, 63, b'-') {
            Ok(Self(value))
        } else {
            Err(IdentifierError {
                kind: "adapter type id",
                value,
            })
        }
    }
}

impl AdapterInstanceId {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        let valid = value.is_ascii()
            && (1..=63).contains(&value.len())
            && value.as_bytes()[0].is_ascii_lowercase()
            && value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'))
            && !value
                .bytes()
                .last()
                .is_some_and(|b| matches!(b, b'-' | b'_'))
            && !value
                .as_bytes()
                .windows(2)
                .any(|pair| matches!(pair, [b'-' | b'_', b'-' | b'_']));
        if valid {
            Ok(Self(value))
        } else {
            Err(IdentifierError {
                kind: "adapter instance id",
                value,
            })
        }
    }
}

impl ConfiguredSource {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        let valid = value.is_ascii()
            && (1..=128).contains(&value.len())
            && value.as_bytes()[0].is_ascii_alphanumeric()
            && value.bytes().all(|b| {
                b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'/' | b'-')
            });
        if valid {
            Ok(Self(value))
        } else {
            Err(IdentifierError {
                kind: "configured source",
                value,
            })
        }
    }
}

fn valid_segmented_lower_id(value: &str, max: usize, separator: u8) -> bool {
    value.is_ascii()
        && (1..=max).contains(&value.len())
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == separator)
        && value.bytes().last() != Some(separator)
        && !value
            .as_bytes()
            .windows(2)
            .any(|pair| pair == [separator, separator])
}

#[derive(Clone)]
pub struct SourceBoundIngest {
    source: ConfiguredSource,
    client: IngestClient,
}

impl fmt::Debug for SourceBoundIngest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SourceBoundIngest")
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

impl SourceBoundIngest {
    pub fn new(source: ConfiguredSource, client: IngestClient) -> Self {
        Self { source, client }
    }

    pub fn source(&self) -> &ConfiguredSource {
        &self.source
    }

    pub fn try_submit(
        &self,
        items: Vec<ReadingItem>,
    ) -> Result<EnqueuedEnvelope, QueueSubmitError> {
        self.client
            .try_submit_with_receipt(iotkit_ingest_client::new_envelope(
                self.source.as_str(),
                items,
            ))
    }

    pub fn try_retry(&self, retry: RetryHandle) -> Result<EnqueuedEnvelope, RetryQueueError> {
        if retry.source() != self.source.as_str() {
            return Err(RetryQueueError::SourceMismatch(retry));
        }
        self.client
            .try_retry_with_receipt(retry)
            .map_err(|error| match error {
                QueueSubmitError::Full(retry) => RetryQueueError::Full(retry),
                QueueSubmitError::Closed(retry) => RetryQueueError::Closed(retry),
            })
    }
}

#[derive(Debug)]
pub enum RetryQueueError {
    Full(RetryHandle),
    Closed(RetryHandle),
    SourceMismatch(RetryHandle),
}

#[derive(Debug, Clone)]
pub struct AdapterStartContext {
    pub instance_id: AdapterInstanceId,
    pub configured_source: ConfiguredSource,
    pub subject_namespace: ConfiguredSource,
    pub ingest: SourceBoundIngest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceBindingMismatch {
    configured_source: ConfiguredSource,
    ingest_source: ConfiguredSource,
}

impl SourceBindingMismatch {
    pub fn configured_source(&self) -> &ConfiguredSource {
        &self.configured_source
    }

    pub fn ingest_source(&self) -> &ConfiguredSource {
        &self.ingest_source
    }
}

impl fmt::Display for SourceBindingMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "configured source {:?} does not match source-bound ingest {:?}",
            self.configured_source, self.ingest_source
        )
    }
}

impl std::error::Error for SourceBindingMismatch {}

impl AdapterStartContext {
    pub fn new(
        instance_id: AdapterInstanceId,
        configured_source: ConfiguredSource,
        ingest: SourceBoundIngest,
    ) -> Self {
        Self::try_new(instance_id, configured_source, ingest)
            .expect("configured source must match source-bound ingest")
    }

    pub fn try_new(
        instance_id: AdapterInstanceId,
        configured_source: ConfiguredSource,
        ingest: SourceBoundIngest,
    ) -> Result<Self, SourceBindingMismatch> {
        if &configured_source != ingest.source() {
            return Err(SourceBindingMismatch {
                configured_source,
                ingest_source: ingest.source().clone(),
            });
        }
        Ok(Self {
            instance_id,
            subject_namespace: configured_source.clone(),
            configured_source,
            ingest,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalTransportKind {
    Serial,
    I2c,
    Spi,
    Gpio,
    Network,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputAdapterTypeDescriptor {
    pub adapter_type_id: AdapterTypeId,
    pub adapter_api_major: u16,
    pub config_schema_version: u16,
    pub implementation_version: &'static str,
    pub display_name: &'static str,
    pub physical_transport_kind: PhysicalTransportKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticKind {
    Transport,
    Protocol,
    Decode,
    MeasurementMapping,
    ClientQueueFull,
    ClientClosed,
    DeviceUnavailable,
}

pub const MAX_DIAGNOSTIC_MESSAGE_BYTES: usize = 512;
pub const MAX_DIAGNOSTIC_CODE_BYTES: usize = 128;
pub const REDACTED_DIAGNOSTIC_MESSAGE: &str = "[redacted]";

#[derive(Clone, PartialEq, Eq)]
pub struct AdapterDiagnostic {
    pub kind: DiagnosticKind,
    pub code: Option<String>,
    pub message: String,
}

impl fmt::Debug for AdapterDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sanitized = self.sanitized();
        f.debug_struct("AdapterDiagnostic")
            .field("kind", &sanitized.kind)
            .field("code", &sanitized.code)
            .field("message", &sanitized.message)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagnosticCodeError;

impl fmt::Display for DiagnosticCodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("diagnostic code must be '<adapter-type-id>:<code>' and fit the bounded syntax")
    }
}

impl std::error::Error for DiagnosticCodeError {}

impl AdapterDiagnostic {
    pub fn new(kind: DiagnosticKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            code: None,
            message: sanitize_diagnostic_message(&message.into()),
        }
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Result<Self, DiagnosticCodeError> {
        let code = code.into();
        if !valid_diagnostic_code(&code) {
            return Err(DiagnosticCodeError);
        }
        self.code = Some(code);
        Ok(self)
    }

    fn sanitized(&self) -> Self {
        Self {
            kind: self.kind,
            code: self
                .code
                .as_ref()
                .filter(|code| valid_diagnostic_code(code))
                .cloned(),
            message: sanitize_diagnostic_message(&self.message),
        }
    }
}

fn valid_diagnostic_code(code: &str) -> bool {
    if code.len() > MAX_DIAGNOSTIC_CODE_BYTES {
        return false;
    }
    let Some((namespace, local)) = code.split_once(':') else {
        return false;
    };
    AdapterTypeId::new(namespace).is_ok()
        && (1..=64).contains(&local.len())
        && local.is_ascii()
        && local.as_bytes()[0].is_ascii_alphanumeric()
        && local
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn sanitize_diagnostic_message(message: &str) -> String {
    let mut bounded = if message.len() <= MAX_DIAGNOSTIC_MESSAGE_BYTES {
        message.to_owned()
    } else {
        let mut end = MAX_DIAGNOSTIC_MESSAGE_BYTES;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message[..end].to_owned()
    };
    let lowercase = bounded.to_ascii_lowercase();
    const SECRET_MARKERS: [&str; 12] = [
        "authorization",
        "bearer ",
        "credential",
        "token",
        "iko_",
        "ikd_",
        "passphrase",
        "password",
        "private key",
        "private_key",
        "secret",
        "-----begin",
    ];
    if SECRET_MARKERS
        .iter()
        .any(|marker| lowercase.contains(marker))
    {
        bounded.clear();
        bounded.push_str(REDACTED_DIAGNOSTIC_MESSAGE);
    }
    bounded
}

#[derive(Debug, Clone, Default)]
pub struct ActivitySnapshot {
    pub last_physical_decode: Option<Instant>,
    pub last_queue_admission: Option<Instant>,
    pub dropped_diagnostics: u64,
}

#[derive(Clone)]
pub struct ActivityReporter {
    state: Arc<Mutex<ActivitySnapshot>>,
}

#[derive(Clone)]
pub struct ActivityReader {
    state: Arc<Mutex<ActivitySnapshot>>,
}

impl ActivityReporter {
    pub fn physical_decode(&self) {
        self.state
            .lock()
            .expect("activity lock poisoned")
            .last_physical_decode = Some(Instant::now());
    }

    pub fn queue_admission(&self) {
        self.state
            .lock()
            .expect("activity lock poisoned")
            .last_queue_admission = Some(Instant::now());
    }

    fn diagnostic_dropped(&self) {
        let mut state = self.state.lock().expect("activity lock poisoned");
        state.dropped_diagnostics = state.dropped_diagnostics.saturating_add(1);
    }
}

impl ActivityReader {
    pub fn snapshot(&self) -> ActivitySnapshot {
        self.state.lock().expect("activity lock poisoned").clone()
    }
}

#[derive(Clone)]
pub struct DiagnosticReporter {
    tx: mpsc::Sender<AdapterDiagnostic>,
    activity: ActivityReporter,
}

impl DiagnosticReporter {
    pub fn try_emit(&self, diagnostic: AdapterDiagnostic) -> Result<(), AdapterDiagnostic> {
        self.tx.try_send(diagnostic.sanitized()).map_err(|error| {
            self.activity.diagnostic_dropped();
            error.into_inner()
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnexpectedExitReason {
    TransportClosed,
    WorkerReturned,
    ClientClosed,
    InternalInvariant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdapterCompletion {
    RequestedStop,
    UnexpectedExit(UnexpectedExitReason),
    Panic,
}

pub struct CompletionReporter {
    tx: Option<oneshot::Sender<AdapterCompletion>>,
}

impl CompletionReporter {
    pub fn complete(mut self, outcome: AdapterCompletion) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(outcome);
        }
    }
}

impl Drop for CompletionReporter {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(AdapterCompletion::UnexpectedExit(
                UnexpectedExitReason::InternalInvariant,
            ));
        }
    }
}

pub struct CompletionHandle {
    rx: oneshot::Receiver<AdapterCompletion>,
}

impl CompletionHandle {
    pub async fn wait(self) -> AdapterCompletion {
        self.rx.await.unwrap_or(AdapterCompletion::Panic)
    }
}

#[derive(Clone)]
pub struct ShutdownHandle {
    tx: watch::Sender<bool>,
}

impl ShutdownHandle {
    pub fn request(&self) {
        self.tx.send_if_modified(|requested| {
            if *requested {
                false
            } else {
                *requested = true;
                true
            }
        });
    }
}

pub struct StopReceiver {
    rx: watch::Receiver<bool>,
}

impl StopReceiver {
    pub fn is_requested(&self) -> bool {
        *self.rx.borrow()
    }

    pub async fn changed(&mut self) -> bool {
        self.rx.changed().await.is_ok() && self.is_requested()
    }
}

pub struct AdapterRuntimeEndpoint {
    pub activity: ActivityReporter,
    pub diagnostics: DiagnosticReporter,
    pub completion: CompletionReporter,
    pub stop: StopReceiver,
}

pub struct RunningInputAdapter {
    pub instance_id: AdapterInstanceId,
    pub activity: ActivityReader,
    pub diagnostics: mpsc::Receiver<AdapterDiagnostic>,
    pub completion: CompletionHandle,
    pub shutdown: ShutdownHandle,
}

pub fn runtime_channels(
    instance_id: AdapterInstanceId,
    diagnostic_capacity: usize,
) -> (AdapterRuntimeEndpoint, RunningInputAdapter) {
    let state = Arc::new(Mutex::new(ActivitySnapshot::default()));
    let activity = ActivityReporter {
        state: Arc::clone(&state),
    };
    let (diagnostic_tx, diagnostic_rx) = mpsc::channel(diagnostic_capacity.max(1));
    let (completion_tx, completion_rx) = oneshot::channel();
    let (stop_tx, stop_rx) = watch::channel(false);
    (
        AdapterRuntimeEndpoint {
            activity: activity.clone(),
            diagnostics: DiagnosticReporter {
                tx: diagnostic_tx,
                activity,
            },
            completion: CompletionReporter {
                tx: Some(completion_tx),
            },
            stop: StopReceiver { rx: stop_rx },
        },
        RunningInputAdapter {
            instance_id,
            activity: ActivityReader { state },
            diagnostics: diagnostic_rx,
            completion: CompletionHandle { rx: completion_rx },
            shutdown: ShutdownHandle { tx: stop_tx },
        },
    )
}

#[cfg(test)]
#[path = "../tests/unit/lib_tests.rs"]
mod tests;
