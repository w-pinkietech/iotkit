use std::{fmt, path::PathBuf};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub(crate) mod integer {
    use serde::{Deserialize, de::Deserializer};
    use serde_json::{Number, Value};

    fn number<'de, D>(deserializer: D) -> Result<Number, D::Error>
    where
        D: Deserializer<'de>,
    {
        match Value::deserialize(deserializer)? {
            Value::Number(number) => Ok(number),
            _ => Err(serde::de::Error::custom("expected integer")),
        }
    }

    fn integral_float(number: &Number) -> Option<f64> {
        let value = number.as_f64()?;
        (value.is_finite() && value.fract() == 0.0).then_some(value)
    }

    pub(crate) fn u32<'de, D>(deserializer: D) -> Result<u32, D::Error>
    where
        D: Deserializer<'de>,
    {
        let number = number(deserializer)?;
        if let Some(value) = number.as_u64()
            && let Ok(value) = u32::try_from(value)
        {
            return Ok(value);
        }
        if let Some(value) = integral_float(&number)
            && (0.0..4_294_967_296.0).contains(&value)
        {
            return Ok(value as u32);
        }
        Err(serde::de::Error::custom("expected u32 integer"))
    }

    pub(crate) fn i64<'de, D>(deserializer: D) -> Result<i64, D::Error>
    where
        D: Deserializer<'de>,
    {
        let number = number(deserializer)?;
        if let Some(value) = number.as_i64() {
            return Ok(value);
        }
        if let Some(value) = number.as_u64()
            && let Ok(value) = i64::try_from(value)
        {
            return Ok(value);
        }
        if let Some(value) = integral_float(&number)
            && (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&value)
        {
            return Ok(value as i64);
        }
        Err(serde::de::Error::custom("expected i64 integer"))
    }

    pub(crate) fn u64<'de, D>(deserializer: D) -> Result<u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        let number = number(deserializer)?;
        if let Some(value) = number.as_u64() {
            return Ok(value);
        }
        if let Some(value) = integral_float(&number)
            && (0.0..18_446_744_073_709_551_616.0).contains(&value)
        {
            return Ok(value as u64);
        }
        Err(serde::de::Error::custom("expected u64 integer"))
    }

    pub(crate) fn usize<'de, D>(deserializer: D) -> Result<usize, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = u64(deserializer)?;
        usize::try_from(value).map_err(|_| serde::de::Error::custom("expected usize integer"))
    }
}

pub const NODE_BACKUP_SUFFIX: &str = ".iotkit-node-backup";
pub const NODE_BACKUP_FORMAT_VERSION: u32 = 1;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryHandoff {
    pub schema_version: u32,
    pub recovery_id: String,
    pub edge_id: String,
    pub edge_node_id: String,
    pub old_ledger_epoch: String,
    pub expected_backup_id: Option<String>,
    pub proposed_new_epoch: String,
    pub credential_generation: i64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupConfig {
    pub schema_version: u32,
    pub database: PathBuf,
    pub destination: PathBuf,
    pub staging_directory: PathBuf,
    pub passphrase_file: PathBuf,
    pub expected_mount: MountIdentity,
    pub freshness_seconds: u64,
    pub retention_count: u32,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MountIdentity {
    pub mount_point: PathBuf,
    pub source: String,
    pub filesystem_type: String,
    pub filesystem_id: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeBackupManifest {
    pub artifact_kind: String,
    #[serde(deserialize_with = "integer::u32")]
    pub format_version: u32,
    pub backup_id: String,
    pub edge_node_id: String,
    pub ledger_epoch: String,
    #[serde(deserialize_with = "integer::i64")]
    pub created_at_ms: i64,
    #[serde(deserialize_with = "integer::i64")]
    pub accepted_cursor: i64,
    #[serde(deserialize_with = "integer::i64")]
    pub allocation_high_water: i64,
    pub snapshot_mode: SnapshotMode,
    pub shutdown_seal_id: Option<String>,
    #[serde(deserialize_with = "integer::u32")]
    pub schema_version: u32,
    #[serde(deserialize_with = "integer::u64")]
    pub database_length: u64,
    pub database_sha256: String,
    pub counts: BackupCounts,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotMode {
    Online,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupCounts {
    #[serde(deserialize_with = "integer::u64")]
    pub devices: u64,
    #[serde(deserialize_with = "integer::u64")]
    pub series: u64,
    #[serde(deserialize_with = "integer::u64")]
    pub readings: u64,
    #[serde(deserialize_with = "integer::u64")]
    pub publication_rows: u64,
    #[serde(deserialize_with = "integer::u64")]
    pub ingest_dedup_rows: u64,
    #[serde(deserialize_with = "integer::u64")]
    pub staged_readings: u64,
    #[serde(deserialize_with = "integer::u64")]
    pub quarantine_rows: u64,
    #[serde(deserialize_with = "integer::u64")]
    pub device_principals: u64,
    #[serde(deserialize_with = "integer::u64")]
    pub device_credentials: u64,
    #[serde(deserialize_with = "integer::u64")]
    pub activation_rows: u64,
    #[serde(deserialize_with = "integer::u64")]
    pub ledger_events: u64,
    #[serde(deserialize_with = "integer::u64")]
    pub audit_events: u64,
}

#[derive(Clone, PartialEq, Eq)]
pub enum RecoveryStartupMode {
    Normal,
    FencedCandidate {
        recovery_id: String,
        candidate_instance_id: String,
        backup_id: Option<String>,
        edge_id: String,
        old_ledger_epoch: String,
        proposed_new_epoch: String,
        credential_generation: i64,
    },
}

#[derive(Clone, PartialEq, Eq)]
pub enum BackupReadiness {
    NotConfigured,
    Healthy {
        artifact: BackupStatusArtifact,
    },
    Stale {
        artifact: BackupStatusArtifact,
    },
    Failed {
        reason_code: String,
        observed_at_ms: i64,
        last_verified: Option<BackupStatusArtifact>,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupStatusArtifact {
    pub backup_id: String,
    pub edge_node_id: String,
    pub ledger_epoch: String,
    pub created_at_ms: i64,
    pub artifact_length: u64,
    pub accepted_cursor: i64,
    pub allocation_high_water: i64,
}

pub struct RestoreRequest {
    pub input: PathBuf,
    pub live_database: PathBuf,
    pub candidate_database: PathBuf,
    pub staging_directory: PathBuf,
    pub handoff: RecoveryHandoff,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreReceipt {
    pub schema_version: u32,
    pub status: RestoreStatus,
    pub recovery_id: String,
    pub candidate_instance_id: String,
    pub backup_id: String,
    pub edge_id: String,
    pub edge_node_id: String,
    pub old_ledger_epoch: String,
    pub proposed_new_epoch: String,
    pub credential_generation: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestoreStatus {
    DurablyFencedCandidate,
}

pub struct BackupPassphrase(Zeroizing<String>);

impl BackupPassphrase {
    pub fn new(secret: String) -> Self {
        Self(Zeroizing::new(secret))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    pub(crate) fn char_count(&self) -> usize {
        self.0.chars().count()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryError {
    InvalidStartupState,
    InvalidSnapshot,
    Storage,
    ContainerInvalid,
    AuthenticationFailed,
    ManifestInvalid,
    DestinationExists,
    Cryptography,
    Random,
    InvalidPassphrase,
    StorageFull,
    ArtifactPublicationUncertain,
}

impl RecoveryError {
    pub fn reason_code(self) -> &'static str {
        match self {
            Self::InvalidStartupState => "invalid_startup_state",
            Self::InvalidSnapshot => "snapshot_invalid",
            Self::Storage => "storage",
            Self::ContainerInvalid => "container_invalid",
            Self::AuthenticationFailed => "authentication_failed",
            Self::ManifestInvalid => "manifest_invalid",
            Self::DestinationExists => "destination_exists",
            Self::Cryptography => "cryptography",
            Self::Random => "random",
            Self::InvalidPassphrase => "passphrase_invalid",
            Self::StorageFull => "storage_full",
            Self::ArtifactPublicationUncertain => "artifact_publication_uncertain",
        }
    }
}

impl fmt::Display for RecoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidStartupState => {
                formatter.write_str("Edge Node recovery startup state is invalid")
            }
            Self::InvalidSnapshot => formatter.write_str("Edge Node snapshot is invalid"),
            Self::Storage => formatter.write_str("Edge Node recovery storage is unavailable"),
            Self::ContainerInvalid => formatter.write_str("Edge Node backup container is invalid"),
            Self::AuthenticationFailed => {
                formatter.write_str("Edge Node backup authentication failed")
            }
            Self::ManifestInvalid => formatter.write_str("Edge Node backup manifest is invalid"),
            Self::DestinationExists => formatter.write_str("Edge Node backup destination exists"),
            Self::Cryptography => formatter.write_str("Edge Node backup cryptography failed"),
            Self::Random => formatter.write_str("Edge Node backup randomness failed"),
            Self::InvalidPassphrase => {
                formatter.write_str("Edge Node backup passphrase is invalid")
            }
            Self::StorageFull => {
                formatter.write_str("Edge Node backup storage capacity is insufficient")
            }
            Self::ArtifactPublicationUncertain => {
                formatter.write_str("Edge Node backup publication status is uncertain")
            }
        }
    }
}

impl std::error::Error for RecoveryError {}

impl From<rusqlite::Error> for RecoveryError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}

impl From<std::io::Error> for RecoveryError {
    fn from(_: std::io::Error) -> Self {
        Self::Storage
    }
}

impl fmt::Debug for RecoveryHandoff {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecoveryHandoff")
            .field("schema_version", &self.schema_version)
            .finish()
    }
}

impl fmt::Debug for BackupConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackupConfig")
            .field("schema_version", &self.schema_version)
            .finish()
    }
}

impl fmt::Debug for MountIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MountIdentity")
    }
}

impl fmt::Debug for NodeBackupManifest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeBackupManifest")
            .field("artifact_kind", &self.artifact_kind)
            .field("format_version", &self.format_version)
            .field("snapshot_mode", &self.snapshot_mode)
            .field("schema_version", &self.schema_version)
            .field("counts", &self.counts)
            .finish()
    }
}

impl fmt::Debug for RecoveryStartupMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Normal => formatter.write_str("RecoveryStartupMode::Normal"),
            Self::FencedCandidate { .. } => {
                formatter.write_str("RecoveryStartupMode::FencedCandidate")
            }
        }
    }
}

impl fmt::Debug for BackupReadiness {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConfigured => formatter.write_str("BackupReadiness::NotConfigured"),
            Self::Healthy { .. } => formatter.write_str("BackupReadiness::Healthy"),
            Self::Stale { .. } => formatter.write_str("BackupReadiness::Stale"),
            Self::Failed { .. } => formatter.write_str("BackupReadiness::Failed"),
        }
    }
}

impl fmt::Debug for BackupStatusArtifact {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BackupStatusArtifact")
    }
}

impl fmt::Debug for RestoreRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RestoreRequest")
    }
}

impl fmt::Debug for RestoreReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RestoreReceipt")
            .field("schema_version", &self.schema_version)
            .field("status", &self.status)
            .finish()
    }
}

impl fmt::Debug for BackupPassphrase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BackupPassphrase([REDACTED])")
    }
}

#[cfg(test)]
#[path = "../tests/unit/model_tests.rs"]
mod tests;
