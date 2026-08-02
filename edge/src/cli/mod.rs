//! Clap parsing and thin dispatch for local operator journeys.

pub mod commands;

use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Deserialize;
use serde_json::json;

use crate::{
    application::{
        accounts::AccountService,
        cli_compat::{
            CliCompatibilityError, CliQueries, LegacyMappingSpec, LegacyMappings, LegacyRoutes,
            LegacyTriggerMode,
        },
        recovery::{
            BackupInspection, BrokerFenceReceipt, RecoveryApplicationError, RestoreReceipt,
        },
    },
    auth::{password::Password, principal::AccountRole},
    backup::{
        create_encrypted_backup, restore_encrypted_backup_postgres, restore_encrypted_backup_sqlite,
    },
    composition::{
        generic_output_adapter,
        runtime::{ProductionRuntimeFactory, run_runtime, shutdown_signal},
        runtime_config::RuntimeConfig,
    },
    diagnostics::{diagnostics_with_certificate, storage_status},
    lifecycle::ExitReason,
    recovery_control::{
        DEFAULT_RECOVERY_CONTROL_SOCKET, RecoveryControlRequest, RecoveryControlResponse,
        call_recovery_control,
    },
    storage::{Storage, StorageProfile, migrate_sqlite_to_postgres},
};

#[derive(Debug, Parser)]
#[command(name = "iotkit-edge", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the IoTKit Edge server.
    Serve(Box<ServeArgs>),
    /// Create or restore an encrypted operational backup.
    Backup {
        #[command(subcommand)]
        command: BackupCommand,
    },
    /// Prepare, authorize, or report a fenced Edge Node recovery.
    Recovery {
        #[command(subcommand)]
        command: RecoveryCommand,
    },
    /// Report storage, custody, and recovery diagnostics as JSON.
    Diagnose(DiagnoseArgs),
    /// Report the storage capacity view as JSON.
    Capacity(DiagnoseArgs),
    /// Bootstrap or recover a local system administrator.
    Account {
        #[command(subcommand)]
        command: AccountCommand,
    },
    /// Offline storage profile operations.
    Storage {
        #[command(subcommand)]
        command: StorageCommand,
    },
    /// List accepted raw measurement records.
    Query(QueryArgs),
    /// Create or revise the legacy production-pulse mapping view.
    MappingSet(MappingSetArgs),
    /// Retire a legacy production-pulse mapping.
    MappingDeactivate(MappingDeactivateArgs),
    /// List legacy production-pulse mapping revisions.
    MappingList(StorageArgs),
    /// Add an exact QoS 1 MQTT route for a legacy mapping.
    RouteAdd(RouteAddArgs),
    /// List legacy MQTT route delivery status.
    RouteList(StorageArgs),
    /// List projected legacy semantic events.
    SemanticQuery(QueryArgs),
}

#[derive(Debug, Subcommand)]
pub enum StorageCommand {
    Migrate(StorageMigrateArgs),
}

#[derive(Debug, Args)]
pub struct StorageMigrateArgs {
    #[arg(long)]
    pub from_sqlite: PathBuf,
    #[arg(long)]
    pub to_postgres_config: PathBuf,
    #[arg(long)]
    pub report: PathBuf,
}

#[derive(Debug, Args)]
pub struct QueryArgs {
    #[command(flatten)]
    pub storage: StorageArgs,
    #[arg(long, default_value_t = 100)]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct MappingSetArgs {
    #[command(flatten)]
    pub storage: StorageArgs,
    #[arg(long)]
    pub edge_node_id: String,
    #[arg(long)]
    pub series_key: String,
    #[arg(long)]
    pub meaning: String,
    #[arg(long)]
    pub trigger_mode: String,
    #[arg(long)]
    pub active_value: i32,
}

#[derive(Debug, Args)]
pub struct MappingDeactivateArgs {
    #[command(flatten)]
    pub storage: StorageArgs,
    #[arg(long)]
    pub edge_node_id: String,
    #[arg(long)]
    pub series_key: String,
}

#[derive(Debug, Args)]
pub struct RouteAddArgs {
    #[command(flatten)]
    pub storage: StorageArgs,
    #[arg(long)]
    pub mapping_id: String,
    #[arg(long)]
    pub topic: String,
}

#[derive(Debug, Subcommand)]
pub enum BackupCommand {
    Create(BackupCreateArgs),
    Restore(BackupRestoreArgs),
    AcceptArchiveLoss(AcceptArchiveLossArgs),
}

#[derive(Debug, Subcommand)]
pub enum RecoveryCommand {
    Prepare(RecoveryPrepareArgs),
    Authorize(RecoveryAuthorizeArgs),
    Report(RecoveryReportArgs),
}

#[derive(Debug, Args)]
pub struct RecoveryPrepareArgs {
    #[arg(long, default_value = DEFAULT_RECOVERY_CONTROL_SOCKET)]
    pub control_socket: PathBuf,
    #[arg(long)]
    pub backup_inspection: PathBuf,
    #[arg(long)]
    pub broker_fence_receipt: PathBuf,
    #[arg(long)]
    pub handoff_output: PathBuf,
}

#[derive(Debug, Args)]
pub struct RecoveryAuthorizeArgs {
    #[arg(long, default_value = DEFAULT_RECOVERY_CONTROL_SOCKET)]
    pub control_socket: PathBuf,
    #[arg(long)]
    pub restore_receipt: PathBuf,
}

#[derive(Debug, Args)]
pub struct RecoveryReportArgs {
    #[arg(long, default_value = DEFAULT_RECOVERY_CONTROL_SOCKET)]
    pub control_socket: PathBuf,
    #[arg(long)]
    pub recovery_id: String,
}

#[derive(Debug, Args)]
pub struct ServeArgs {
    #[command(flatten)]
    pub storage: StorageArgs,
    #[arg(long)]
    pub edge_id: String,
    #[arg(long)]
    pub broker_url: String,
    #[arg(long, default_value = "iotkit-edge")]
    pub client_id: String,
    #[arg(long)]
    pub username: String,
    #[arg(long)]
    pub password_file: PathBuf,
    #[arg(long)]
    pub trust_mode: Option<String>,
    #[arg(long)]
    pub ca_file: Option<PathBuf>,
    #[arg(long)]
    pub allow_insecure: bool,
    #[arg(long, default_value = "127.0.0.1:8080")]
    pub http_listen: String,
    #[arg(long)]
    pub public_origin: String,
    #[arg(long, env = "IOTKIT_DISPLAY_TIME_ZONE", default_value = "UTC")]
    pub display_time_zone: String,
    #[arg(long)]
    pub development_http: bool,
    #[arg(long, value_enum, default_value_t = DeploymentProfileArg::Field)]
    pub deployment_profile: DeploymentProfileArg,
    #[arg(long)]
    pub broker_certificate_file: Option<PathBuf>,
    #[arg(long, default_value_t = 90)]
    pub storage_warning_percent: i32,
    #[arg(long, default_value = DEFAULT_RECOVERY_CONTROL_SOCKET)]
    pub recovery_control_socket: PathBuf,
    #[arg(long)]
    pub output_broker_url: Option<String>,
    #[arg(long, default_value = "iotkit-edge-output")]
    pub output_client_id: String,
    #[arg(long)]
    pub output_username: Option<String>,
    #[arg(long)]
    pub output_password_file: Option<PathBuf>,
    #[arg(long)]
    pub output_trust_mode: Option<String>,
    #[arg(long)]
    pub output_ca_file: Option<PathBuf>,
    #[arg(long)]
    pub output_allow_insecure: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DeploymentProfileArg {
    Field,
    Trial,
}

#[derive(Debug, Args)]
pub struct BackupCreateArgs {
    #[command(flatten)]
    pub storage: StorageArgs,
    #[arg(long = "output")]
    pub output: PathBuf,
    #[arg(long = "passphrase-file")]
    pub passphrase_file: PathBuf,
}

#[derive(Debug, Args)]
pub struct BackupRestoreArgs {
    #[command(flatten)]
    pub storage: StorageArgs,
    #[arg(long = "input")]
    pub input: PathBuf,
    #[arg(long = "passphrase-file")]
    pub passphrase_file: PathBuf,
}

#[derive(Debug, Args)]
pub struct AcceptArchiveLossArgs {
    #[command(flatten)]
    pub storage: StorageArgs,
    #[arg(long)]
    pub edge_node_id: String,
    #[arg(long)]
    pub ledger_epoch: String,
    #[arg(long)]
    pub confirm_edge_id: String,
    #[arg(long)]
    pub reason: String,
}

#[derive(Debug, Args)]
pub struct DiagnoseArgs {
    #[command(flatten)]
    pub storage: StorageArgs,
    #[arg(long = "storage-warning-percent", default_value_t = 90)]
    pub storage_warning_percent: i32,
    #[arg(long = "broker-certificate-file")]
    pub broker_certificate_file: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum AccountCommand {
    Bootstrap(AccountBootstrapArgs),
    Recover(AccountRecoverArgs),
}

#[derive(Debug, Args)]
pub struct AccountBootstrapArgs {
    #[command(flatten)]
    pub storage: StorageArgs,
    #[arg(long = "login-id")]
    pub login_id: String,
    #[arg(long = "display-name")]
    pub display_name: String,
    #[arg(long = "password-file")]
    pub password_file: PathBuf,
}

#[derive(Debug, Args)]
pub struct AccountRecoverArgs {
    #[command(flatten)]
    pub storage: StorageArgs,
    #[arg(long = "login-id")]
    pub login_id: String,
    #[arg(long = "password-file")]
    pub password_file: PathBuf,
}

#[derive(Debug, Clone, Args)]
pub struct StorageArgs {
    #[arg(long = "storage-profile", value_enum, default_value_t = StorageProfileArg::Embedded)]
    pub profile: StorageProfileArg,
    #[arg(long = "db", default_value = "edge.db")]
    pub database: PathBuf,
    #[arg(long = "postgres-config")]
    pub postgres_config: Option<PathBuf>,
    #[arg(long = "storage-metadata")]
    pub storage_metadata: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum StorageProfileArg {
    Embedded,
    Postgres,
}

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("secret file must be an owner-only regular file")]
    SecretPermissions,
    #[error("secret file is empty")]
    EmptySecret,
    #[error("invalid PostgreSQL configuration file")]
    PostgresConfiguration,
    #[error("configured storage profile does not match deployment metadata")]
    ProfileMetadata,
    #[error("configured storage profile does not match deployment")]
    ExpectedProfile,
    #[error("PostgreSQL configuration is not allowed for embedded storage")]
    UnexpectedPostgresConfiguration,
    #[error("--postgres-config is required for postgres storage")]
    MissingPostgresConfiguration,
    #[error("migration report already exists")]
    MigrationReportExists,
    #[error("--from-sqlite must name an existing Edge database")]
    MigrationSource,
    #[error(
        "usage: iotkit-edge <serve|account|backup|storage|diagnose|capacity|query|mapping-set|mapping-deactivate|mapping-list|route-add|route-list|semantic-query> [options]"
    )]
    Usage,
    #[error("serve configuration is invalid: {0}")]
    ServeConfiguration(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Storage(#[from] crate::storage::StorageError),
    #[error(transparent)]
    Backup(#[from] crate::backup::BackupError),
    #[error(transparent)]
    Diagnostic(#[from] crate::diagnostics::DiagnosticError),
    #[error(transparent)]
    Account(#[from] crate::application::accounts::AccountApplicationError),
    #[error(transparent)]
    Password(#[from] crate::auth::password::PasswordError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    RuntimeConfig(#[from] crate::composition::runtime_config::RuntimeConfigError),
    #[error(transparent)]
    Runtime(#[from] crate::composition::runtime::RuntimeError),
    #[error(transparent)]
    CliCompatibility(#[from] CliCompatibilityError),
    #[error(transparent)]
    Recovery(#[from] RecoveryApplicationError),
    #[error("recovery control request failed: {0}")]
    RecoveryControl(String),
}

pub async fn run(cli: Cli) -> Result<ExitReason, CliError> {
    match cli.command.ok_or(CliError::Usage)? {
        Command::Serve(args) => {
            args.validate()?;
            let config = RuntimeConfig::from_serve_args(&args)?;
            let shutdown = shutdown_signal()?;
            run_runtime(config, &ProductionRuntimeFactory, shutdown)
                .await
                .map_err(Into::into)
        }
        Command::Backup { command } => {
            match command {
                BackupCommand::Create(args) => {
                    let passphrase = read_owner_only_secret(&args.passphrase_file)?;
                    let storage = args.storage.connect().await?;
                    let manifest =
                        create_encrypted_backup(&storage, &args.output, &passphrase).await?;
                    write_json(&manifest)?;
                }
                BackupCommand::Restore(args) => {
                    let passphrase = read_owner_only_secret(&args.passphrase_file)?;
                    args.storage.validate_profile()?;
                    let manifest = match args.storage.profile {
                        StorageProfileArg::Embedded => {
                            restore_encrypted_backup_sqlite(
                                &args.input,
                                &args.storage.database,
                                &passphrase,
                            )
                            .await?
                        }
                        StorageProfileArg::Postgres => {
                            let dsn = args.storage.postgres_dsn()?;
                            restore_encrypted_backup_postgres(&args.input, &dsn, &passphrase)
                                .await?
                        }
                    };
                    write_json(&manifest)?;
                }
                BackupCommand::AcceptArchiveLoss(args) => {
                    let storage = args.storage.connect().await?;
                    storage
                        .accept_restored_archive_loss(
                            &args.edge_node_id,
                            &args.ledger_epoch,
                            &args.confirm_edge_id,
                            &args.reason,
                            unix_milliseconds()?,
                        )
                        .await?;
                    write_json(&json!({
                        "status": "archive_lost",
                        "edge_node_id": args.edge_node_id,
                        "ledger_epoch": args.ledger_epoch,
                    }))?;
                }
            }
            Ok(ExitReason::Requested)
        }
        Command::Recovery { command } => {
            match command {
                RecoveryCommand::Prepare(args) => {
                    let inspection: BackupInspection =
                        serde_json::from_str(&read_owner_only_secret(&args.backup_inspection)?)?;
                    let fence: BrokerFenceReceipt =
                        serde_json::from_str(&read_owner_only_secret(&args.broker_fence_receipt)?)?;
                    let handoff = match call_recovery_control(
                        &args.control_socket,
                        &RecoveryControlRequest::Prepare { inspection, fence },
                    )
                    .await
                    .map_err(|error| CliError::RecoveryControl(error.to_string()))?
                    {
                        RecoveryControlResponse::Prepared { handoff } => handoff,
                        RecoveryControlResponse::Rejected { code } => {
                            return Err(CliError::RecoveryControl(code));
                        }
                        _ => return Err(CliError::RecoveryControl("unexpected_response".into())),
                    };
                    write_owner_only_json_atomic(&args.handoff_output, &handoff)?;
                    write_json(&json!({
                        "status": "prepared",
                        "recovery_id": handoff.recovery_id,
                        "edge_node_id": handoff.edge_node_id,
                        "old_ledger_epoch": handoff.old_ledger_epoch,
                        "new_ledger_epoch": handoff.proposed_new_epoch,
                        "credential_generation": handoff.credential_generation,
                    }))?;
                }
                RecoveryCommand::Authorize(args) => {
                    let receipt: RestoreReceipt =
                        serde_json::from_str(&read_owner_only_secret(&args.restore_receipt)?)?;
                    let request = match call_recovery_control(
                        &args.control_socket,
                        &RecoveryControlRequest::Authorize { receipt },
                    )
                    .await
                    .map_err(|error| CliError::RecoveryControl(error.to_string()))?
                    {
                        RecoveryControlResponse::Authorized { request } => request,
                        RecoveryControlResponse::Rejected { code } => {
                            return Err(CliError::RecoveryControl(code));
                        }
                        _ => return Err(CliError::RecoveryControl("unexpected_response".into())),
                    };
                    write_json(&json!({
                        "status": "authorized",
                        "recovery_id": request.recovery_id,
                        "edge_node_id": request.edge_node_id,
                        "candidate_instance_id": request.candidate_instance_id,
                        "new_ledger_epoch": request.new_ledger_epoch,
                        "broker_credential_generation": request.broker_credential_generation,
                        "device_auth_generation": request.device_auth_generation,
                    }))?;
                }
                RecoveryCommand::Report(args) => {
                    let report = match call_recovery_control(
                        &args.control_socket,
                        &RecoveryControlRequest::Report {
                            recovery_id: args.recovery_id,
                        },
                    )
                    .await
                    .map_err(|error| CliError::RecoveryControl(error.to_string()))?
                    {
                        RecoveryControlResponse::Report { report } => report,
                        RecoveryControlResponse::Rejected { code } => {
                            return Err(CliError::RecoveryControl(code));
                        }
                        _ => return Err(CliError::RecoveryControl("unexpected_response".into())),
                    };
                    write_json(&report)?;
                }
            }
            Ok(ExitReason::Requested)
        }
        Command::Diagnose(args) => {
            let storage = args.storage.connect().await?;
            let report = diagnostics_with_certificate(
                &storage,
                args.storage_warning_percent,
                unix_milliseconds()?,
                args.broker_certificate_file.as_deref(),
            )
            .await?;
            write_json(&report)?;
            Ok(ExitReason::Requested)
        }
        Command::Capacity(args) => {
            let storage = args.storage.connect().await?;
            write_json(&storage_status(&storage, args.storage_warning_percent).await?)?;
            Ok(ExitReason::Requested)
        }
        Command::Account { command } => {
            match command {
                AccountCommand::Bootstrap(args) => {
                    let password = Password::new(read_owner_only_secret(&args.password_file)?)?;
                    let storage = args.storage.connect().await?;
                    let account = AccountService::new(storage)
                        .create_initial_system_admin(
                            &args.login_id,
                            &args.display_name,
                            password,
                            unix_milliseconds()?,
                        )
                        .await?;
                    write_account(&account)?;
                }
                AccountCommand::Recover(args) => {
                    let password = Password::new(read_owner_only_secret(&args.password_file)?)?;
                    let storage = args.storage.connect().await?;
                    let account = AccountService::new(storage)
                        .recover_system_admin_password(
                            &args.login_id,
                            password,
                            unix_milliseconds()?,
                        )
                        .await?;
                    write_account(&account)?;
                }
            }
            Ok(ExitReason::Requested)
        }
        Command::Storage { command } => {
            match command {
                StorageCommand::Migrate(args) => {
                    let metadata = fs::symlink_metadata(&args.from_sqlite)
                        .map_err(|_| CliError::MigrationSource)?;
                    if !metadata.file_type().is_file() {
                        return Err(CliError::MigrationSource);
                    }
                    if fs::symlink_metadata(&args.report).is_ok() {
                        return Err(CliError::MigrationReportExists);
                    }
                    let dsn = read_postgres_dsn(&args.to_postgres_config)?;
                    let report = migrate_sqlite_to_postgres(&args.from_sqlite, &dsn).await?;
                    write_owner_only_json_atomic(&args.report, &report)?;
                }
            }
            Ok(ExitReason::Requested)
        }
        Command::Query(args) => {
            let storage = args.storage.connect().await?;
            write_json(&CliQueries::new(storage).raw_records(args.limit).await?)?;
            Ok(ExitReason::Requested)
        }
        Command::MappingSet(args) => {
            let trigger_mode = match args.trigger_mode.as_str() {
                "active_sample" => LegacyTriggerMode::ActiveSample,
                "active_edge" => LegacyTriggerMode::ActiveEdge,
                _ => {
                    return Err(CliCompatibilityError::InvalidMapping(
                        "trigger mode must be active_sample or active_edge".into(),
                    )
                    .into());
                }
            };
            let storage = args.storage.connect().await?;
            let mapping = LegacyMappings::new(storage)
                .put(
                    LegacyMappingSpec {
                        edge_node_id: args.edge_node_id,
                        series_key: args.series_key,
                        meaning: args.meaning,
                        trigger_mode,
                        active_value: args.active_value,
                    },
                    unix_milliseconds()?,
                )
                .await?;
            write_json(&mapping)?;
            Ok(ExitReason::Requested)
        }
        Command::MappingDeactivate(args) => {
            let storage = args.storage.connect().await?;
            let mapping = LegacyMappings::new(storage)
                .deactivate(&args.edge_node_id, &args.series_key, unix_milliseconds()?)
                .await?;
            write_json(&mapping)?;
            Ok(ExitReason::Requested)
        }
        Command::MappingList(args) => {
            let storage = args.connect().await?;
            write_json(&LegacyMappings::new(storage).list().await?)?;
            Ok(ExitReason::Requested)
        }
        Command::RouteAdd(args) => {
            let storage = args.storage.connect().await?;
            let route = LegacyRoutes::new(storage, generic_output_adapter())
                .add(&args.mapping_id, &args.topic, unix_milliseconds()?)
                .await?;
            write_json(&route)?;
            Ok(ExitReason::Requested)
        }
        Command::RouteList(args) => {
            let storage = args.connect().await?;
            write_json(
                &LegacyRoutes::new(storage, generic_output_adapter())
                    .list()
                    .await?,
            )?;
            Ok(ExitReason::Requested)
        }
        Command::SemanticQuery(args) => {
            let storage = args.storage.connect().await?;
            write_json(&CliQueries::new(storage).semantic_events(args.limit).await?)?;
            Ok(ExitReason::Requested)
        }
    }
}

impl ServeArgs {
    fn validate(&self) -> Result<(), CliError> {
        self.storage.validate_profile()?;
        if !(50..=99).contains(&self.storage_warning_percent) {
            return Err(CliError::ServeConfiguration(
                "storage warning percent must be between 50 and 99".into(),
            ));
        }
        Ok(())
    }
}

impl StorageArgs {
    async fn connect(&self) -> Result<Storage, CliError> {
        Storage::connect(self.storage_profile()?)
            .await
            .map_err(Into::into)
    }

    pub(crate) fn storage_profile(&self) -> Result<StorageProfile, CliError> {
        self.validate_profile()?;
        Ok(match self.profile {
            StorageProfileArg::Embedded => StorageProfile::Sqlite {
                path: self.database.clone(),
            },
            StorageProfileArg::Postgres => StorageProfile::Postgres {
                dsn: self.postgres_dsn()?,
            },
        })
    }

    fn validate_profile(&self) -> Result<(), CliError> {
        let profile = match self.profile {
            StorageProfileArg::Embedded => "embedded",
            StorageProfileArg::Postgres => "postgres",
        };
        if let Ok(expected) = std::env::var("IOTKIT_EXPECTED_STORAGE_PROFILE")
            && expected != profile
        {
            return Err(CliError::ExpectedProfile);
        }
        if let Some(path) = &self.storage_metadata {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Metadata {
                profile: String,
            }
            let metadata: Metadata =
                serde_json::from_slice(&fs::read(path)?).map_err(|_| CliError::ProfileMetadata)?;
            if metadata.profile != profile {
                return Err(CliError::ProfileMetadata);
            }
        }
        if self.profile == StorageProfileArg::Embedded
            && self
                .postgres_config
                .as_ref()
                .is_some_and(|path| !path.as_os_str().is_empty())
        {
            return Err(CliError::UnexpectedPostgresConfiguration);
        }
        Ok(())
    }

    fn postgres_dsn(&self) -> Result<String, CliError> {
        let path = self
            .postgres_config
            .as_ref()
            .filter(|path| !path.as_os_str().is_empty())
            .ok_or(CliError::MissingPostgresConfiguration)?;
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Configuration {
            dsn: String,
        }
        let encoded = read_owner_only_secret(path)?;
        let configuration: Configuration =
            serde_json::from_str(&encoded).map_err(|_| CliError::PostgresConfiguration)?;
        if configuration.dsn.is_empty() {
            return Err(CliError::PostgresConfiguration);
        }
        Ok(configuration.dsn)
    }
}

pub(crate) fn read_owner_only_secret(path: &Path) -> Result<String, CliError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.permissions().mode() & 0o077 != 0 {
        return Err(CliError::SecretPermissions);
    }
    let mut value = fs::read_to_string(path)?;
    if value.ends_with('\n') {
        value.pop();
        if value.ends_with('\r') {
            value.pop();
        }
    }
    if value.is_empty() {
        return Err(CliError::EmptySecret);
    }
    Ok(value)
}

fn write_json(value: &impl serde::Serialize) -> Result<(), CliError> {
    let mut output = std::io::stdout().lock();
    serde_json::to_writer(&mut output, value)?;
    output.write_all(b"\n")?;
    Ok(())
}

fn read_postgres_dsn(path: &Path) -> Result<String, CliError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Configuration {
        dsn: String,
    }
    let encoded = read_owner_only_secret(path)?;
    let configuration: Configuration =
        serde_json::from_str(&encoded).map_err(|_| CliError::PostgresConfiguration)?;
    if configuration.dsn.is_empty() {
        return Err(CliError::PostgresConfiguration);
    }
    Ok(configuration.dsn)
}

fn write_owner_only_json_atomic(
    path: &Path,
    value: &impl serde::Serialize,
) -> Result<(), CliError> {
    if fs::symlink_metadata(path).is_ok() {
        return Err(CliError::MigrationReportExists);
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let temporary = parent.join(format!(
        ".iotkit-edge-report-{}-{}",
        std::process::id(),
        unix_milliseconds()?
    ));
    let result = (|| -> Result<(), CliError> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        serde_json::to_writer_pretty(&mut file, value)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        fs::hard_link(&temporary, path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                CliError::MigrationReportExists
            } else {
                CliError::Io(error)
            }
        })?;
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    let _ = fs::remove_file(temporary);
    result
}

fn write_account(account: &crate::storage::Account) -> Result<(), CliError> {
    let role = match account.role {
        AccountRole::Viewer => "viewer",
        AccountRole::Admin => "admin",
        AccountRole::SystemAdmin => "system_admin",
    };
    write_json(&json!({
        "account_ref": account.account_ref,
        "login_id": account.login_id,
        "display_name": account.display_name,
        "role": role,
        "must_change_password": account.must_change_password,
        "revision": account.revision,
    }))
}

fn unix_milliseconds() -> Result<i64, CliError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| std::io::Error::other("system clock is before the Unix epoch"))?;
    i64::try_from(duration.as_millis())
        .map_err(|_| std::io::Error::other("system clock cannot be represented").into())
}
