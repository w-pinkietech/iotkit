mod cmd {
    pub mod backup;
    pub mod device_credential;
    pub mod devices;
    pub mod fingerprint;
    pub mod passphrase;
    pub mod query;
    pub mod registry;
    pub mod replace;
    pub mod smoke;
    pub mod snapshot;
    pub mod target;
    pub mod time;
    pub mod token;
}

use clap::{Parser, Subcommand};
use std::path::PathBuf;

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Parser)]
#[command(name = "iotkit-edge-nodectl", version)]
struct Cli {
    #[arg(long)]
    db: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Init,
    Identity,
    MqttBinding,
    Smoke {
        #[command(subcommand)]
        command: cmd::smoke::SmokeCommand,
    },
    Sightings {
        #[command(subcommand)]
        command: SightingsCommand,
    },
    Device {
        #[command(subcommand)]
        command: DeviceCommand,
    },
    DeviceCredential {
        #[command(subcommand)]
        command: cmd::device_credential::DeviceCredentialCommand,
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
    Backup {
        #[command(subcommand)]
        command: cmd::backup::BackupCommand,
    },
    Target {
        #[command(subcommand)]
        command: cmd::target::TargetCommand,
    },
    Passphrase {
        #[command(subcommand)]
        command: cmd::passphrase::PassphraseCommand,
    },
    Time {
        #[command(subcommand)]
        command: cmd::time::TimeCommand,
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
    if let Command::Backup { command } = cli.command {
        return cmd::backup::run(command).map_err(|error| error.into());
    }
    let initializing = matches!(&cli.command, Command::Init);
    let reading_identity = matches!(&cli.command, Command::Identity | Command::MqttBinding);
    let reading_smoke_status = matches!(
        &cli.command,
        Command::Smoke {
            command: cmd::smoke::SmokeCommand::Status(_),
        }
    );
    let restoring_snapshot = matches!(
        &cli.command,
        Command::Snapshot {
            command: cmd::snapshot::SnapshotCommand::Restore(_),
        }
    );
    let restore_target = match &cli.command {
        Command::Snapshot {
            command: cmd::snapshot::SnapshotCommand::Restore(args),
        } => Some(args.db.clone()),
        Command::Snapshot {
            command: cmd::snapshot::SnapshotCommand::RestoreStatus(args),
        } => Some(args.db.clone()),
        _ => None,
    };
    let allow_missing_db = initializing
        || matches!(
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
    let database_existed_before_open = db_path.exists();
    if !database_existed_before_open && !allow_missing_db {
        return Err(format!("database file does not exist: {}", db_path.display()).into());
    }
    iotkit_core_storage::preflight_edge_node_database(&db_path)?;
    if reading_identity || reading_smoke_status {
        let conn = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let identity = load_initialized_identity(&conn)?;
        return match cli.command {
            Command::Identity => write_identity(&identity),
            Command::MqttBinding => {
                let binding =
                    iotkit_core_publish::mqtt::MqttBinding::for_edge_node(&identity.edge_node_id)?;
                println!("{}", serde_json::to_string_pretty(&binding)?);
                Ok(())
            }
            Command::Smoke {
                command: cmd::smoke::SmokeCommand::Status(args),
            } => cmd::smoke::run_status(&conn, &identity, args),
            _ => unreachable!("read-only command match was checked"),
        };
    }
    if matches!(
        &cli.command,
        Command::Snapshot {
            command: cmd::snapshot::SnapshotCommand::RestoreStatus(_),
        }
    ) {
        let conn = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        return cmd::snapshot::run_restore_status(&conn);
    }
    if matches!(
        &cli.command,
        Command::DeviceCredential {
            command: cmd::device_credential::DeviceCredentialCommand::List(_),
        }
    ) {
        let conn = rusqlite::Connection::open_with_flags(
            &db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        if let Command::DeviceCredential { command } = cli.command {
            return cmd::device_credential::run(&conn, command);
        }
        unreachable!("device credential list match was checked");
    }
    let mut created_target = None;
    if allow_missing_db {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&db_path)
            .map_err(|error| {
                if initializing {
                    format!(
                        "init requires an absent database ({}): {error}",
                        db_path.display()
                    )
                } else {
                    format!(
                        "restore --create requires an absent target created exclusively by restore ({}): {error}",
                        db_path.display()
                    )
                }
            })?;
        created_target = Some(CreatedDatabaseTarget::new(db_path.clone()));
    }

    // Snapshot keeps its established JSON wire shape, but it operates on the
    // same current Edge Node schema as every other command. Opening a legacy
    // database therefore applies the current migrations before dispatch.
    let all_migrations = iotkit_core_recovery::all_edge_node_migrations();

    let db = iotkit_core_storage::init_db(&db_path, &all_migrations)?;
    if !restoring_snapshot {
        ensure_edge_node_id(&db)?;
    }
    if created_target.is_none() {
        reconcile_database_initialization_provenance(&db, &db_path, database_existed_before_open)?;
    }
    db.with_conn_sync(|conn| Ok(dispatch(conn, &db_path, cli.command)))??;
    if restoring_snapshot {
        // Restore validates that the target is pristine, so identity is minted only after the
        // restore transaction has committed successfully.
        ensure_edge_node_id(&db)?;
    }
    if created_target.is_some() {
        // The restore transaction must validate the exclusively created database while it is
        // pristine and establish local recovery itself. Reconcile only afterward to create a
        // missing marker without interpreting a pre-existing marker as state in the fresh DB.
        reconcile_database_initialization_provenance(&db, &db_path, true)?;
    }
    if let Some(target) = created_target.as_mut() {
        target.committed = true;
    }
    Ok(())
}

fn ensure_edge_node_id(
    db: &iotkit_core_storage::DbHandle,
) -> Result<(), iotkit_core_storage::StorageError> {
    db.with_conn_sync(|conn| {
        iotkit_core_ledger::edge_node_id(conn)
            .map(|_| ())
            .map_err(ledger_to_storage_err)
    })
}

fn load_initialized_identity(
    conn: &rusqlite::Connection,
) -> AppResult<iotkit_core_ledger::EdgeNodeIdentity> {
    let has_ledger_meta = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='ledger_meta')",
        [],
        |row| row.get::<_, bool>(0),
    )?;
    if !has_ledger_meta {
        return Err("Edge Node identity is not initialized; create a new database with `iotkit-edge-nodectl --db <path> init`".into());
    }
    match iotkit_core_ledger::load_edge_node_identity(conn) {
        Ok(identity) => Ok(identity),
        Err(iotkit_core_ledger::LedgerError::NotFound(_)) => Err(
            "Edge Node identity is not initialized; create a new database with `iotkit-edge-nodectl --db <path> init`"
                .into(),
        ),
        Err(error) => Err(error.into()),
    }
}

fn write_identity(identity: &iotkit_core_ledger::EdgeNodeIdentity) -> AppResult<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "edge_node_id": identity.edge_node_id,
            "ledger_epoch": identity.ledger_epoch,
        }))?
    );
    Ok(())
}

fn ledger_to_storage_err(
    error: iotkit_core_ledger::LedgerError,
) -> iotkit_core_storage::StorageError {
    iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(Box::new(
        error,
    )))
}

fn reconcile_database_initialization_provenance(
    db: &iotkit_core_storage::DbHandle,
    db_path: &std::path::Path,
    database_existed_before_open: bool,
) -> Result<(), iotkit_core_storage::StorageError> {
    db.with_conn_sync(|conn| {
        iotkit_core_ops::reconcile_database_initialization_provenance(
            conn,
            db_path,
            database_existed_before_open,
        )
        .map_err(|error| {
            iotkit_core_storage::StorageError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
                Box::new(error),
            ))
        })
    })
}

struct CreatedDatabaseTarget {
    path: PathBuf,
    committed: bool,
    marker_existed: bool,
}

impl CreatedDatabaseTarget {
    fn new(path: PathBuf) -> Self {
        let marker_existed = iotkit_core_ops::database_initialization_marker_path(&path).exists();
        Self {
            path,
            committed: false,
            marker_existed,
        }
    }
}

impl Drop for CreatedDatabaseTarget {
    fn drop(&mut self) {
        let committed_receipt = rusqlite::Connection::open_with_flags(
            &self.path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .and_then(|conn| {
            conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM restore_receipts WHERE id = 1)",
                [],
                |row| row.get::<_, bool>(0),
            )
        })
        .unwrap_or(false);
        if self.committed || committed_receipt {
            return;
        }
        let mut paths = vec![
            self.path.clone(),
            PathBuf::from(format!("{}-wal", self.path.display())),
            PathBuf::from(format!("{}-shm", self.path.display())),
        ];
        if !self.marker_existed {
            paths.push(iotkit_core_ops::database_initialization_marker_path(
                &self.path,
            ));
        }
        for path in paths {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    eprintln!(
                        "failed to clean database target {}: {error}",
                        path.display()
                    )
                }
            }
        }
    }
}

fn dispatch(
    conn: &rusqlite::Connection,
    db_path: &std::path::Path,
    command: Command,
) -> AppResult<()> {
    match command {
        Command::Init => {
            let edge_node_id = iotkit_core_ledger::edge_node_id(conn)?;
            let ledger_epoch = iotkit_core_ledger::ledger_epoch(conn)?;
            write_identity(&iotkit_core_ledger::EdgeNodeIdentity {
                edge_node_id,
                ledger_epoch,
            })
        }
        Command::Identity | Command::MqttBinding => {
            unreachable!("read-only identity commands do not enter the write dispatcher")
        }
        Command::Smoke { command } => match command {
            cmd::smoke::SmokeCommand::Enqueue => cmd::smoke::run_enqueue(conn),
            cmd::smoke::SmokeCommand::Status(_) => {
                unreachable!("read-only smoke status does not enter the write dispatcher")
            }
        },
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
        Command::DeviceCredential { command } => cmd::device_credential::run(conn, command),
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
            cmd::snapshot::SnapshotCommand::RestoreStatus(_) => {
                cmd::snapshot::run_restore_status(conn)
            }
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
        Command::Time { command } => match command {
            cmd::time::TimeCommand::Confirm => cmd::time::run_time_confirm(conn),
        },
        Command::Token { command } => match command {
            cmd::token::TokenCommand::Issue(args) => cmd::token::run_token_issue(conn, args),
            cmd::token::TokenCommand::Revoke(args) => cmd::token::run_token_revoke(conn, args),
            cmd::token::TokenCommand::List => cmd::token::run_token_list(conn),
        },
        Command::Fingerprint => cmd::fingerprint::run_fingerprint(conn, db_path),
        Command::Health(args) => cmd::query::run_health(db_path, args),
        Command::Backup { .. } => unreachable!("backup commands take the early route"),
    }
}
