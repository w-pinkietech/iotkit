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
    auth::{
        password::{Password, hash_password},
        principal::AccountRole,
    },
    backup::{
        create_encrypted_backup, restore_encrypted_backup_postgres, restore_encrypted_backup_sqlite,
    },
    diagnostics::{diagnostics, storage_status},
    lifecycle::ExitReason,
    storage::{AuditActor, Storage, StorageProfile},
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
    Serve,
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
pub struct DiagnoseArgs {
    #[command(flatten)]
    pub storage: StorageArgs,
    #[arg(long = "storage-warning-percent", default_value_t = 90)]
    pub storage_warning_percent: i32,
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
    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => Ok(Application::new().run().await),
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
            }
            Ok(ExitReason::Requested)
        }
        Command::Diagnose(args) => {
            let storage = args.storage.connect().await?;
            let report =
                diagnostics(&storage, args.storage_warning_percent, unix_milliseconds()?).await?;
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
                    let credential = storage
                        .get_account_credential_by_login(&args.login_id)
                        .await?;
                    let account = storage
                        .replace_account_password(
                            &credential.account.account_ref,
                            credential.account.revision,
                            hash_password(&password)?,
                            false,
                            AuditActor::local_cli(),
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
        if self.profile == StorageProfileArg::Embedded && self.postgres_config.is_some() {
            return Err(CliError::UnexpectedPostgresConfiguration);
        }
        Ok(())
    }

    fn postgres_dsn(&self) -> Result<String, CliError> {
        let path = self
            .postgres_config
            .as_ref()
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
