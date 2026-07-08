mod cmd {
    pub mod devices;
    pub mod fingerprint;
    pub mod passphrase;
    pub mod query;
    pub mod registry;
    pub mod replace;
    pub mod snapshot;
    pub mod target;
    pub mod token;
}

use clap::{Parser, Subcommand};
use std::path::PathBuf;

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Parser)]
#[command(name = "gatewayctl")]
struct Cli {
    #[arg(long)]
    db: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Sightings {
        #[command(subcommand)]
        command: SightingsCommand,
    },
    Device {
        #[command(subcommand)]
        command: DeviceCommand,
    },
    Events {
        #[command(subcommand)]
        command: EventsCommand,
    },
    Readings {
        #[command(subcommand)]
        command: ReadingsCommand,
    },
    Registry {
        #[command(subcommand)]
        command: RegistryCommand,
    },
    Series {
        #[command(subcommand)]
        command: SeriesCommand,
    },
    Snapshot {
        #[command(subcommand)]
        command: cmd::snapshot::SnapshotCommand,
    },
    Target {
        #[command(subcommand)]
        command: cmd::target::TargetCommand,
    },
    Passphrase {
        #[command(subcommand)]
        command: cmd::passphrase::PassphraseCommand,
    },
    Token {
        #[command(subcommand)]
        command: cmd::token::TokenCommand,
    },
    Fingerprint,
    Health(cmd::query::HealthArgs),
}

#[derive(Subcommand)]
enum SightingsCommand {
    List,
}

#[derive(Subcommand)]
enum DeviceCommand {
    List(cmd::devices::ListArgs),
    Add(cmd::devices::AddArgs),
    Approve(cmd::devices::ApproveArgs),
    Activate(cmd::devices::SystemIdArgs),
    Retire(cmd::devices::RetireArgs),
    Replace(cmd::replace::ReplaceArgs),
    ReplaceUndo(cmd::replace::ReplaceUndoArgs),
}

#[derive(Subcommand)]
enum EventsCommand {
    Tail(cmd::devices::TailEventsArgs),
}

#[derive(Subcommand)]
enum ReadingsCommand {
    Query(cmd::query::QueryArgs),
    Aggregate(cmd::query::AggregateArgs),
    Export(cmd::query::ExportArgs),
}

#[derive(Subcommand)]
enum RegistryCommand {
    List(cmd::registry::RegistryListArgs),
    Enable(cmd::registry::RegistryEnableArgs),
    Alias(cmd::registry::RegistryAliasArgs),
}

#[derive(Subcommand)]
enum SeriesCommand {
    List(cmd::registry::SeriesListArgs),
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn run() -> AppResult<()> {
    let cli = Cli::parse();
    let restore_target = match &cli.command {
        Command::Snapshot {
            command: cmd::snapshot::SnapshotCommand::Restore(args),
        } => Some(args.db.clone()),
        _ => None,
    };
    let allow_missing_db = matches!(
        &cli.command,
        Command::Snapshot {
            command: cmd::snapshot::SnapshotCommand::Restore(args),
        } if args.create
    );
    let db_path = restore_target
        .or(cli.db)
        .or_else(|| std::env::var_os("IOTKIT_DB_PATH").map(PathBuf::from));
    let Some(db_path) = db_path else {
        return Err("no database specified: pass --db or set IOTKIT_DB_PATH".into());
    };
    if !db_path.exists() && !allow_missing_db {
        return Err(format!("database file does not exist: {}", db_path.display()).into());
    }

    let mut all_migrations = iotkit_core_storage::MIGRATIONS.to_vec();
    all_migrations.extend_from_slice(iotkit_core_ledger::MIGRATIONS); // v3, v5, v9, v11
    all_migrations.extend_from_slice(iotkit_core_timeseries::MIGRATIONS); // v4, v7, v8
    all_migrations.extend_from_slice(iotkit_core_registry::MIGRATIONS); // v6
    all_migrations.extend_from_slice(iotkit_core_publish::MIGRATIONS); // v10
    all_migrations.extend_from_slice(iotkit_core_ops::MIGRATIONS); // v12
    all_migrations.sort_by_key(|m| m.version); // 1,3,4,5,6,7,8,9,10,11,12

    let db = iotkit_core_storage::init_db(&db_path, &all_migrations)?;
    db.with_conn_sync(|conn| Ok(dispatch(conn, &db_path, cli.command)))?
}

fn dispatch(
    conn: &rusqlite::Connection,
    db_path: &std::path::Path,
    command: Command,
) -> AppResult<()> {
    match command {
        Command::Sightings { command } => match command {
            SightingsCommand::List => cmd::devices::run_list_sightings(conn),
        },
        Command::Device { command } => match command {
            DeviceCommand::List(args) => cmd::devices::run_list_devices(conn, args),
            DeviceCommand::Add(args) => cmd::devices::run_add_device(conn, args),
            DeviceCommand::Approve(args) => cmd::devices::run_approve_device(conn, args),
            DeviceCommand::Activate(args) => cmd::devices::run_activate_device(conn, args),
            DeviceCommand::Retire(args) => cmd::devices::run_retire_device(conn, args),
            DeviceCommand::Replace(args) => cmd::replace::run_replace(conn, args),
            DeviceCommand::ReplaceUndo(args) => cmd::replace::run_replace_undo(conn, args),
        },
        Command::Events { command } => match command {
            EventsCommand::Tail(args) => cmd::devices::run_tail_events(conn, args),
        },
        Command::Readings { command } => match command {
            ReadingsCommand::Query(args) => cmd::query::run_query(conn, args),
            ReadingsCommand::Aggregate(args) => cmd::query::run_aggregate(conn, args),
            ReadingsCommand::Export(args) => cmd::query::run_export(conn, args),
        },
        Command::Registry { command } => match command {
            RegistryCommand::List(args) => cmd::registry::run_registry_list(conn, args),
            RegistryCommand::Enable(args) => cmd::registry::run_registry_enable(conn, args),
            RegistryCommand::Alias(args) => cmd::registry::run_registry_alias(conn, args),
        },
        Command::Series { command } => match command {
            SeriesCommand::List(args) => cmd::registry::run_series_list(conn, args),
        },
        Command::Snapshot { command } => match command {
            cmd::snapshot::SnapshotCommand::Export(args) => cmd::snapshot::run_export(conn, args),
            cmd::snapshot::SnapshotCommand::Restore(args) => cmd::snapshot::run_restore(conn, args),
        },
        Command::Target { command } => {
            let real_smoke = |endpoint: &str, token: &str| -> Result<(), String> {
                let response = reqwest::blocking::Client::new()
                    .post(endpoint)
                    .bearer_auth(token)
                    .json(&serde_json::json!({
                        "publication_id": "smoke",
                        "records": [],
                    }))
                    .send()
                    .map_err(|e| e.to_string())?;
                if !response.status().is_success() {
                    return Err(format!(
                        "smoke POST returned non-success status: {}",
                        response.status()
                    ));
                }
                let ack: serde_json::Value = response
                    .json()
                    .map_err(|e| format!("smoke ack decode failed: {e}"))?;
                if ack.get("publication_id").and_then(|v| v.as_str()) != Some("smoke") {
                    return Err("smoke ack did not echo publication_id \"smoke\"".to_string());
                }
                Ok(())
            };
            match command {
                cmd::target::TargetCommand::Add(args) => cmd::target::run_target_add(
                    conn,
                    &args.endpoint,
                    &args.token,
                    args.schema_version,
                    &real_smoke,
                ),
                cmd::target::TargetCommand::List => cmd::target::run_target_list(conn),
                cmd::target::TargetCommand::RotateToken(args) => {
                    cmd::target::run_target_rotate_token(conn, &args.token, &real_smoke)
                }
                cmd::target::TargetCommand::Remove(args) => {
                    cmd::target::run_target_remove(conn, args.abandon_custody)
                }
            }
        }
        Command::Passphrase { command } => match command {
            cmd::passphrase::PassphraseCommand::Reset => {
                cmd::passphrase::run_passphrase_reset(conn)
            }
        },
        Command::Token { command } => match command {
            cmd::token::TokenCommand::Issue(args) => cmd::token::run_token_issue(conn, args),
            cmd::token::TokenCommand::Revoke(args) => cmd::token::run_token_revoke(conn, args),
            cmd::token::TokenCommand::List => cmd::token::run_token_list(conn),
        },
        Command::Fingerprint => cmd::fingerprint::run_fingerprint(conn, db_path),
        Command::Health(args) => cmd::query::run_health(db_path, args),
    }
}
