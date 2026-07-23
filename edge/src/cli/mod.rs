//! Clap parsing and thin dispatch for local operator journeys.

pub mod commands;

use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Deserialize;
use serde_json::json;

use crate::{
    Application,
    application::accounts::AccountService,
    auth::{password::Password, principal::AccountRole},
    backup::{
        create_encrypted_backup, restore_encrypted_backup_postgres, restore_encrypted_backup_sqlite,
    },
    diagnostics::{diagnostics_with_certificate, storage_status},
    lifecycle::ExitReason,
    storage::{Storage, StorageProfile},
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
    /// Report storage, custody, and recovery diagnostics as JSON.
    Diagnose(DiagnoseArgs),
    /// Report the storage capacity view as JSON.
    Capacity(DiagnoseArgs),
    /// Bootstrap or recover a local system administrator.
    Account {
        #[command(subcommand)]
        command: AccountCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum BackupCommand {
    Create(BackupCreateArgs),
    Restore(BackupRestoreArgs),
    AcceptArchiveLoss(AcceptArchiveLossArgs),
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
    #[arg(long)]
    pub development_http: bool,
    #[arg(long)]
    pub broker_certificate_file: Option<PathBuf>,
    #[arg(long, default_value_t = 90)]
    pub storage_warning_percent: i32,
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
    #[error("usage: iotkit-edge <serve|account|backup|diagnose|capacity> [options]")]
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
}

pub async fn run(cli: Cli) -> Result<ExitReason, CliError> {
    match cli.command.ok_or(CliError::Usage)? {
        Command::Serve(args) => {
            args.validate()?;
            Ok(Application::new().run().await)
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
    }
}

impl ServeArgs {
    fn validate(&self) -> Result<(), CliError> {
        self.storage.validate_profile()?;
        if self.edge_id.is_empty()
            || self.broker_url.is_empty()
            || self.username.is_empty()
            || self.public_origin.is_empty()
        {
            return Err(CliError::ServeConfiguration(
                "edge ID, broker URL, username, and public origin are required".into(),
            ));
        }
        let _password = read_owner_only_secret(&self.password_file)?;
        if self.allow_insecure && (self.trust_mode.is_some() || self.ca_file.is_some()) {
            return Err(CliError::ServeConfiguration(
                "allow-insecure conflicts with TLS trust options".into(),
            ));
        }
        if !self.allow_insecure && self.trust_mode.is_none() {
            return Err(CliError::ServeConfiguration(
                "trust-mode is required for broker TLS".into(),
            ));
        }
        if self.output_broker_url.is_some() {
            if self.output_username.as_deref().unwrap_or("").is_empty()
                || self.output_password_file.is_none()
            {
                return Err(CliError::ServeConfiguration(
                    "output username and password file are required".into(),
                ));
            }
            let _output_password =
                read_owner_only_secret(self.output_password_file.as_deref().unwrap())?;
            if !self.output_allow_insecure && self.output_trust_mode.is_none() {
                return Err(CliError::ServeConfiguration(
                    "output trust-mode is required for TLS".into(),
                ));
            }
        }
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
        self.validate_profile()?;
        Storage::connect(match self.profile {
            StorageProfileArg::Embedded => StorageProfile::Sqlite {
                path: self.database.clone(),
            },
            StorageProfileArg::Postgres => StorageProfile::Postgres {
                dsn: self.postgres_dsn()?,
            },
        })
        .await
        .map_err(Into::into)
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

fn read_owner_only_secret(path: &Path) -> Result<String, CliError> {
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
