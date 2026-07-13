use clap::{Args, ValueEnum};
use iotkit_core_ledger as ledger;
use rusqlite::{Connection, TransactionBehavior};

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum KindArg {
    Individual,
    Positional,
}

impl From<KindArg> for ledger::DeviceKind {
    fn from(value: KindArg) -> Self {
        match value {
            KindArg::Individual => Self::Individual,
            KindArg::Positional => Self::Positional,
        }
    }
}

#[derive(Args)]
pub struct ListArgs {
    #[arg(long)]
    pub all: bool,
}

#[derive(Args)]
pub struct AddArgs {
    #[arg(long)]
    pub hardware_id: String,
    #[arg(long)]
    pub label: Option<String>,
    #[arg(long, value_enum, default_value_t = KindArg::Individual)]
    pub kind: KindArg,
    #[arg(long)]
    pub active: bool,
    #[arg(long, default_value = "default")]
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
pub struct ApproveArgs {
    pub hardware_id: String,
    #[arg(long)]
    pub label: Option<String>,
    #[arg(long, value_enum, default_value_t = KindArg::Individual)]
    pub kind: KindArg,
}

#[derive(Args)]
pub struct SystemIdArgs {
    pub system_id_text: String,
}

#[derive(Args)]
pub struct RetireArgs {
    pub system_id_text: String,
    #[arg(long)]
    pub yes: bool,
}

#[derive(Args)]
pub struct TailEventsArgs {
    #[arg(long, default_value_t = 20)]
    pub limit: u32,
}

fn state_label(state: ledger::DeviceState) -> &'static str {
    match state {
        ledger::DeviceState::Quarantined => "quarantined",
        ledger::DeviceState::Active => "active",
        ledger::DeviceState::Retired => "retired",
    }
}

fn kind_label(kind: ledger::DeviceKind) -> &'static str {
    match kind {
        ledger::DeviceKind::Individual => "individual",
        ledger::DeviceKind::Positional => "positional",
    }
}

pub(crate) fn mutate<T, F>(conn: &Connection, f: F) -> AppResult<T>
where
    F: FnOnce(&rusqlite::Transaction<'_>) -> AppResult<T>,
{
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let value = f(&tx)?;
    ledger::bump_generation(&tx)?;
    tx.commit()?;
    Ok(value)
}

pub fn run_list_sightings(conn: &Connection) -> AppResult<()> {
    for row in ledger::list_sightings(conn)? {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            row.hardware_id, row.source, row.observations, row.first_seen, row.last_seen
        );
    }
    Ok(())
}

pub fn run_list_devices(conn: &Connection, args: ListArgs) -> AppResult<()> {
    for row in ledger::list_devices(conn, args.all)? {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            row.system_id.to_text(),
            row.hardware_id,
            state_label(row.state),
            kind_label(row.kind),
            row.user_label.unwrap_or_default()
        );
    }
    Ok(())
}

pub fn run_add_device(conn: &Connection, args: AddArgs) -> AppResult<()> {
    if args.active {
        return Err("device add with a credential always starts quarantined; activate it separately after validation".into());
    }
    if !matches!(args.kind, KindArg::Individual) {
        return Err("device credential registration supports individual devices only".into());
    }
    let op = if args.accept_capacity_debt {
        "device.add_with_credential_capacity_debt"
    } else {
        "device.add_with_credential"
    };
    let mut params = serde_json::json!({"hardware_id":args.hardware_id,"label":args.label,"flow_class":args.flow_class,"reason_code":"device_commissioning"});
    if args.accept_capacity_debt && !args.dry_run {
        super::device_credential::preview_confirm_and_bind_capacity_debt(
            conn,
            op,
            &mut params,
            args.yes,
        )?;
    }
    let value = iotkit_core_ops::dispatch(
        conn,
        iotkit_core_ops::standard_catalog(),
        iotkit_core_ops::DispatchRequest {
            op: op.into(),
            params,
            dry_run: args.dry_run,
            actor: iotkit_core_ops::Actor {
                actor_id: "local_cli".into(),
                actor_kind: iotkit_core_ops::ActorKind::LocalCli,
                tier_ceiling: iotkit_core_ops::Tier::Construction,
            },
            source: Some("local_cli".into()),
            step_up_verified: args.accept_capacity_debt,
            clock_trust: None,
        },
    )?;
    if let iotkit_core_ops::DispatchResult::DeviceCredential(secret) = value {
        let (metadata, plaintext) = secret.consume();
        eprintln!(
            "system_id: {}",
            metadata["system_id"].as_str().unwrap_or("")
        );
        eprintln!(
            "principal_id: {}",
            metadata["principal_id"].as_str().unwrap_or("")
        );
        eprintln!(
            "credential_id: {}",
            metadata["credential_id"].as_str().unwrap_or("")
        );
        eprintln!("WARNING: this device token is shown once and cannot be displayed again.");
        eprintln!(
            "If this initial token is lost before delivery, revoke it, then issue a new credential with `iotkit-edgectl device-credential issue`."
        );
        println!("{}", plaintext.as_str());
    } else {
        println!("{}", serde_json::to_string(value.metadata())?);
    }
    Ok(())
}

pub fn run_approve_device(conn: &Connection, args: ApproveArgs) -> AppResult<()> {
    let sid = mutate(conn, |tx| {
        Ok(ledger::approve_sighting(
            tx,
            &args.hardware_id,
            args.label.as_deref(),
            args.kind.into(),
        )?)
    })?;
    println!("{}", sid.to_text());
    Ok(())
}

pub fn run_activate_device(conn: &Connection, args: SystemIdArgs) -> AppResult<()> {
    let sid = ledger::SystemId::from_text(&args.system_id_text)?;
    mutate(conn, |tx| {
        ledger::activate_device(tx, &sid)?;
        Ok(())
    })
}

pub fn run_retire_device(conn: &Connection, args: RetireArgs) -> AppResult<()> {
    if !args.yes {
        eprintln!(
            "Retire device {}? Type 'yes' to continue:",
            args.system_id_text
        );
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if line.trim() != "yes" {
            return Err("retire aborted".into());
        }
    }
    let sid = ledger::SystemId::from_text(&args.system_id_text)?;
    mutate(conn, |tx| {
        ledger::retire_device(tx, &sid)?;
        Ok(())
    })
}

pub fn run_tail_events(conn: &Connection, args: TailEventsArgs) -> AppResult<()> {
    for row in ledger::list_recent_events(conn, args.limit)? {
        let system_id = row.system_id.map(|s| s.to_text()).unwrap_or_default();
        println!(
            "{}\t{}\t{}\t{}\t{}",
            row.event_id, row.at, row.kind, system_id, row.detail
        );
    }
    Ok(())
}
