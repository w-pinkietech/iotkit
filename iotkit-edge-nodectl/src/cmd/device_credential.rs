use clap::{Args, Subcommand};
use iotkit_core_ops::{
    Actor, ActorKind, DispatchRequest, DispatchResult, Tier, dispatch, standard_catalog,
};
use rusqlite::Connection;
use serde_json::{Value, json};

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Subcommand)]
pub enum DeviceCredentialCommand {
    Issue(IssueArgs),
    Reissue(PrincipalArgs),
    Confirm(CredentialArgs),
    Abandon(CredentialArgs),
    Revoke(CredentialArgs),
    List(ListArgs),
    FlowClass(FlowClassArgs),
    Configure(ConfigureArgs),
}

#[derive(Args)]
pub struct IssueArgs {
    #[arg(long)]
    pub principal_id: String,
    #[arg(long)]
    pub reason_code: String,
    #[arg(long)]
    pub accept_capacity_debt: bool,
    /// Deliberate noninteractive confirmation for capacity debt automation.
    #[arg(long)]
    pub yes: bool,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args)]
pub struct PrincipalArgs {
    #[arg(long)]
    pub principal_id: String,
    #[arg(long)]
    pub reason_code: String,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args)]
pub struct CredentialArgs {
    #[arg(long)]
    pub principal_id: String,
    #[arg(long)]
    pub credential_id: String,
    #[arg(long)]
    pub reason_code: String,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args)]
pub struct ListArgs {
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args)]
pub struct FlowClassArgs {
    #[arg(long)]
    pub principal_id: String,
    #[arg(long)]
    pub flow_class: String,
    #[arg(long)]
    pub accept_capacity_debt: bool,
    /// Deliberate noninteractive confirmation for capacity debt automation.
    #[arg(long)]
    pub yes: bool,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args)]
pub struct ConfigureArgs {
    #[arg(long)]
    pub low_steady_units: i64,
    #[arg(long)]
    pub low_burst_units: i64,
    #[arg(long)]
    pub default_steady_units: i64,
    #[arg(long)]
    pub default_burst_units: i64,
    #[arg(long)]
    pub high_steady_units: i64,
    #[arg(long)]
    pub high_burst_units: i64,
    #[arg(long)]
    pub capacity_steady_units: i64,
    #[arg(long)]
    pub capacity_burst_units: i64,
    #[arg(long)]
    pub stale_after_ms: i64,
    #[arg(long)]
    pub accept_capacity_debt: bool,
    /// Required for the construction-tier configuration write.
    #[arg(long)]
    pub yes: bool,
    #[arg(long)]
    pub dry_run: bool,
}

fn bind_capacity_approval(params: &mut Value, preview: &Value) -> AppResult<()> {
    let object = params
        .as_object_mut()
        .ok_or("capacity approval params must be an object")?;
    for (expected, shown) in [
        ("expected_required_steady_units", "required_steady_units"),
        ("expected_required_burst_units", "required_burst_units"),
        ("expected_capacity_steady_units", "capacity_steady_units"),
        ("expected_capacity_burst_units", "capacity_burst_units"),
        ("expected_authority_generation", "authority_generation"),
    ] {
        object.insert(
            expected.into(),
            preview
                .get(shown)
                .cloned()
                .ok_or_else(|| format!("capacity preview omitted {shown}"))?,
        );
    }
    Ok(())
}

fn confirm_capacity_debt(preview: &Value, yes: bool) -> AppResult<()> {
    eprintln!(
        "Capacity debt approval: required steady/burst = {}/{}, available = {}/{}.",
        preview["required_steady_units"],
        preview["required_burst_units"],
        preview["capacity_steady_units"],
        preview["capacity_burst_units"],
    );
    if !yes {
        eprintln!("Type 'approve-capacity-debt' to continue:");
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if line.trim() != "approve-capacity-debt" {
            return Err("capacity debt approval aborted; use --yes only for deliberate noninteractive automation".into());
        }
    }
    Ok(())
}

fn capacity_preview_race_hook() -> AppResult<()> {
    let (Ok(ready), Ok(proceed)) = (
        std::env::var("IOTKIT_TEST_CAPACITY_PREVIEW_READY_FILE"),
        std::env::var("IOTKIT_TEST_CAPACITY_PREVIEW_CONTINUE_FILE"),
    ) else {
        return Ok(());
    };
    std::fs::write(&ready, b"ready")?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !std::path::Path::new(&proceed).exists() {
        if std::time::Instant::now() >= deadline {
            return Err("timed out waiting for capacity preview race continuation".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Ok(())
}

pub(crate) fn preview_confirm_and_bind_capacity_debt(
    conn: &Connection,
    op: &str,
    params: &mut Value,
    yes: bool,
) -> AppResult<()> {
    let preview = invoke(conn, op, params.clone(), true, true)?;
    confirm_capacity_debt(preview.metadata(), yes)?;
    capacity_preview_race_hook()?;
    bind_capacity_approval(params, preview.metadata())
}

pub(crate) fn invoke(
    conn: &Connection,
    op: &str,
    params: Value,
    dry_run: bool,
    step_up: bool,
) -> AppResult<DispatchResult> {
    Ok(dispatch(
        conn,
        standard_catalog(),
        DispatchRequest {
            op: op.into(),
            params,
            dry_run,
            actor: Actor {
                actor_id: "local_cli".into(),
                actor_kind: ActorKind::LocalCli,
                tier_ceiling: Tier::Construction,
            },
            source: Some("local_cli".into()),
            step_up_verified: step_up,
            clock_trust: None,
        },
    )?)
}

pub fn run(conn: &Connection, command: DeviceCredentialCommand) -> AppResult<()> {
    let value = match command {
        DeviceCredentialCommand::Issue(args) => {
            let op = if args.accept_capacity_debt {
                "device_credential.issue_capacity_debt"
            } else {
                "device_credential.issue"
            };
            let mut params =
                json!({"principal_id":args.principal_id,"reason_code":args.reason_code});
            if args.accept_capacity_debt && !args.dry_run {
                preview_confirm_and_bind_capacity_debt(conn, op, &mut params, args.yes)?;
            }
            invoke(conn, op, params, args.dry_run, args.accept_capacity_debt)?
        }
        DeviceCredentialCommand::Reissue(args) => invoke(
            conn,
            "device_credential.reissue",
            json!({"principal_id":args.principal_id,"reason_code":args.reason_code}),
            args.dry_run,
            false,
        )?,
        DeviceCredentialCommand::Confirm(args) => invoke(
            conn,
            "device_credential.confirm",
            json!({"principal_id":args.principal_id,"credential_id":args.credential_id,"reason_code":args.reason_code}),
            args.dry_run,
            false,
        )?,
        DeviceCredentialCommand::Abandon(args) => invoke(
            conn,
            "device_credential.abandon",
            json!({"principal_id":args.principal_id,"credential_id":args.credential_id,"reason_code":args.reason_code}),
            args.dry_run,
            false,
        )?,
        DeviceCredentialCommand::Revoke(args) => invoke(
            conn,
            "device_credential.revoke",
            json!({"principal_id":args.principal_id,"credential_id":args.credential_id,"reason_code":args.reason_code}),
            args.dry_run,
            false,
        )?,
        DeviceCredentialCommand::List(_args) => {
            let principals=iotkit_core_ops::list_device_principals(conn)?.into_iter().map(|p|json!({"principal_id":p.principal_id,"device_system_id":p.device_system_id.to_text(),"flow_class":p.flow_class,"profile":p.profile,"scopes":p.scopes.into_iter().map(|s|s.to_text()).collect::<Vec<_>>() })).collect::<Vec<_>>();
            let credentials=iotkit_core_ops::list_device_credentials(conn)?.into_iter().map(|c|json!({"credential_id":c.credential_id,"principal_id":c.principal_id,"state":c.state.as_str(),"issued_at":c.issued_at,"last_used_at":c.last_used_at,"proven_at":c.proven_at,"confirmed_at":c.confirmed_at,"revoked_at":c.revoked_at,"issue_reason":c.issue_reason,"revoke_reason":c.revoke_reason})).collect::<Vec<_>>();
            DispatchResult::Public(json!({"principals":principals,"credentials":credentials}))
        }
        DeviceCredentialCommand::FlowClass(args) => {
            let op = if args.accept_capacity_debt {
                "device.flow_class_change_capacity_debt"
            } else {
                "device.flow_class_change"
            };
            let mut params =
                json!({"principal_ids":[args.principal_id],"flow_class":args.flow_class});
            if args.accept_capacity_debt && !args.dry_run {
                preview_confirm_and_bind_capacity_debt(conn, op, &mut params, args.yes)?;
            }
            invoke(conn, op, params, args.dry_run, args.accept_capacity_debt)?
        }
        DeviceCredentialCommand::Configure(args) => {
            if !args.dry_run && !args.yes {
                return Err(
                    "authority configuration requires --yes after reviewing measured local network values"
                        .into(),
                );
            }
            let op = if args.accept_capacity_debt {
                "device.authority_configure_capacity_debt"
            } else {
                "device.authority_configure"
            };
            let mut params = json!({
                "low_steady_units":args.low_steady_units,"low_burst_units":args.low_burst_units,
                "default_steady_units":args.default_steady_units,"default_burst_units":args.default_burst_units,
                "high_steady_units":args.high_steady_units,"high_burst_units":args.high_burst_units,
                "capacity_steady_units":args.capacity_steady_units,"capacity_burst_units":args.capacity_burst_units,
                "stale_after_ms":args.stale_after_ms,
            });
            if args.accept_capacity_debt && !args.dry_run {
                preview_confirm_and_bind_capacity_debt(conn, op, &mut params, args.yes)?;
            }
            invoke(conn, op, params, args.dry_run, true)?
        }
    };
    if let DispatchResult::DeviceCredential(secret) = value {
        let (metadata, plaintext) = secret.consume();
        if let Some(id) = metadata.get("credential_id").and_then(Value::as_str) {
            eprintln!("credential_id: {id}");
        }
        eprintln!("WARNING: this device token is shown once and cannot be displayed again.");
        match metadata.get("state").and_then(Value::as_str) {
            Some("pending") => eprintln!(
                "If this pending token is lost before delivery, abandon it, then reissue a new credential with `iotkit-edge-nodectl device-credential reissue`."
            ),
            _ => eprintln!(
                "If this current token is lost before delivery, revoke it, then issue a new credential with `iotkit-edge-nodectl device-credential issue`."
            ),
        }
        println!("{}", plaintext.as_str());
    } else {
        println!("{}", serde_json::to_string(value.metadata())?);
    }
    Ok(())
}
