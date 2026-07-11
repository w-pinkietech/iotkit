use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use iotkit_core_ledger::SystemId;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use crate::OpsError;

const TOKEN_BYTES: usize = 32;
const ID_BYTES: usize = 16;
const LAST_USED_UPDATE_INTERVAL_MS: i64 = 60_000;
const HEALTH_COUNT_CAP: usize = 10_000;
pub const DEVICE_PRINCIPAL_SCOPE_CAP: usize = 64;
const CAPACITY_MATH_OVERFLOW: &str = "capacity_math_overflow";
type LifecycleTimes = (i64, Option<i64>, Option<i64>, Option<i64>);
pub const REPLACEMENT_BACKUP_ACTION: &str = "Install Plan 6.5 encrypted replacement backup support, then create a complete encrypted replacement backup.";

pub(crate) trait CredentialEntropy {
    fn fill(&mut self, output: &mut [u8]) -> Result<(), OpsError>;
}

pub(crate) struct SystemCredentialEntropy;

impl CredentialEntropy for SystemCredentialEntropy {
    fn fill(&mut self, output: &mut [u8]) -> Result<(), OpsError> {
        getrandom::fill(output).map_err(|_| OpsError::Random)
    }
}

pub(crate) trait CredentialClock {
    fn now_ms(&self) -> i64;
}

pub(crate) struct SystemCredentialClock;

impl CredentialClock for SystemCredentialClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or(0)
    }
}

/// One-time bearer material. It is non-cloneable, zeroized on drop, and always redacted.
pub struct DeviceCredentialPresentation(Zeroizing<String>);

impl DeviceCredentialPresentation {
    /// Consumes the only presentation handle. The returned buffer remains zeroizing.
    pub fn consume(self) -> Zeroizing<String> {
        self.0
    }
}

impl fmt::Debug for DeviceCredentialPresentation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED DEVICE CREDENTIAL]")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceCredentialState {
    Current,
    Pending,
    Revoked,
}

impl DeviceCredentialState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Pending => "pending",
            Self::Revoked => "revoked",
        }
    }

    fn parse(value: &str) -> Result<Self, OpsError> {
        match value {
            "current" => Ok(Self::Current),
            "pending" => Ok(Self::Pending),
            "revoked" => Ok(Self::Revoked),
            _ => Err(OpsError::Validation(
                "invalid device credential state".into(),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialReasonCode {
    DeviceCommissioning,
    ManualIssue,
    CredentialReissue,
    CredentialConfirmed,
    PendingAbandoned,
    OperatorRevoked,
}

impl CredentialReasonCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DeviceCommissioning => "device_commissioning",
            Self::ManualIssue => "manual_issue",
            Self::CredentialReissue => "credential_reissue",
            Self::CredentialConfirmed => "credential_confirmed",
            Self::PendingAbandoned => "pending_abandoned",
            Self::OperatorRevoked => "operator_revoked",
        }
    }

    pub fn parse(value: &str) -> Result<Self, OpsError> {
        match value {
            "device_commissioning" => Ok(Self::DeviceCommissioning),
            "manual_issue" => Ok(Self::ManualIssue),
            "credential_reissue" => Ok(Self::CredentialReissue),
            "credential_confirmed" => Ok(Self::CredentialConfirmed),
            "pending_abandoned" => Ok(Self::PendingAbandoned),
            "operator_revoked" => Ok(Self::OperatorRevoked),
            _ => Err(OpsError::Validation(
                "unknown credential lifecycle reason code".into(),
            )),
        }
    }

    fn issue_code(self) -> Result<&'static str, OpsError> {
        match self {
            Self::DeviceCommissioning | Self::ManualIssue | Self::CredentialReissue => {
                Ok(self.as_str())
            }
            _ => Err(OpsError::Validation(
                "reason code is not valid for issuance".into(),
            )),
        }
    }

    fn revoke_code(self) -> Result<&'static str, OpsError> {
        match self {
            Self::CredentialConfirmed | Self::PendingAbandoned | Self::OperatorRevoked => {
                Ok(self.as_str())
            }
            _ => Err(OpsError::Validation(
                "reason code is not valid for revocation".into(),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevicePrincipalRow {
    pub principal_id: String,
    pub device_system_id: SystemId,
    pub flow_class: String,
    pub profile: String,
    pub scopes: Vec<SystemId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCredentialRow {
    pub credential_id: String,
    pub principal_id: String,
    pub state: DeviceCredentialState,
    pub issued_at: i64,
    pub last_used_at: Option<i64>,
    pub proven_at: Option<i64>,
    pub confirmed_at: Option<i64>,
    pub revoked_at: Option<i64>,
    pub issue_reason: String,
    pub revoke_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevicePrincipal {
    principal_id: String,
    credential_id: String,
    device_system_id: SystemId,
    scopes: Vec<SystemId>,
    flow_class: String,
    profile: String,
    auth_epoch: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAuthentication {
    principal: DevicePrincipal,
    credential_state: DeviceCredentialState,
    auth_generation: i64,
    principal_material_generation: i64,
}

impl DevicePrincipal {
    pub fn principal_id(&self) -> &str {
        &self.principal_id
    }
    pub fn credential_id(&self) -> &str {
        &self.credential_id
    }
    pub fn device_system_id(&self) -> SystemId {
        self.device_system_id
    }
    pub fn scopes(&self) -> &[SystemId] {
        &self.scopes
    }
    pub fn flow_class(&self) -> &str {
        &self.flow_class
    }
    pub fn profile(&self) -> &str {
        &self.profile
    }
    pub fn auth_epoch(&self) -> &str {
        &self.auth_epoch
    }
}

impl DeviceAuthentication {
    pub fn principal(&self) -> &DevicePrincipal {
        &self.principal
    }
    pub fn credential_state(&self) -> DeviceCredentialState {
        self.credential_state
    }
    pub fn auth_generation(&self) -> i64 {
        self.auth_generation
    }
    pub fn principal_material_generation(&self) -> i64 {
        self.principal_material_generation
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapacityStatus {
    pub required_steady_units: i64,
    pub required_burst_units: i64,
    pub capacity_steady_units: i64,
    pub capacity_burst_units: i64,
}

impl CapacityStatus {
    pub fn exceeds(self) -> bool {
        self.required_steady_units > self.capacity_steady_units
            || self.required_burst_units > self.capacity_burst_units
    }

    fn no_worse_than(self, previous: Self) -> Result<bool, OpsError> {
        let overage = |required: i64, capacity: i64| {
            if required > capacity {
                required.checked_sub(capacity).ok_or_else(capacity_overflow)
            } else {
                Ok(0)
            }
        };
        Ok(
            overage(self.required_steady_units, self.capacity_steady_units)?
                <= overage(
                    previous.required_steady_units,
                    previous.capacity_steady_units,
                )?
                && overage(self.required_burst_units, self.capacity_burst_units)?
                    <= overage(previous.required_burst_units, previous.capacity_burst_units)?,
        )
    }
}

fn capacity_overflow() -> OpsError {
    OpsError::Validation(CAPACITY_MATH_OVERFLOW.into())
}

fn checked_capacity_add(left: i64, right: i64) -> Result<i64, OpsError> {
    left.checked_add(right).ok_or_else(capacity_overflow)
}

fn checked_capacity_sub(left: i64, right: i64) -> Result<i64, OpsError> {
    left.checked_sub(right).ok_or_else(capacity_overflow)
}

fn checked_capacity_mul(left: i64, right: i64) -> Result<i64, OpsError> {
    left.checked_mul(right).ok_or_else(capacity_overflow)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapacityHealth {
    pub status: CapacityStatus,
    pub active_debt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaleCredentialHealth {
    pub active_count: u64,
    pub stale_count: u64,
    pub counts_capped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplacementBackupHealth {
    pub replacement_backup_unavailable: bool,
    pub recovery_action: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlowWeight {
    pub steady_units: i64,
    pub burst_units: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceAuthorityConfig {
    pub low: FlowWeight,
    pub default: FlowWeight,
    pub high: FlowWeight,
    pub capacity: FlowWeight,
    pub stale_after_ms: i64,
}

impl DeviceAuthorityConfig {
    pub fn validate(self) -> Result<Self, OpsError> {
        let values = [
            self.low.steady_units,
            self.low.burst_units,
            self.default.steady_units,
            self.default.burst_units,
            self.high.steady_units,
            self.high.burst_units,
            self.capacity.steady_units,
            self.capacity.burst_units,
            self.stale_after_ms,
        ];
        if values.into_iter().any(|value| value <= 0) {
            return Err(OpsError::Validation(
                "authority configuration values must be positive".into(),
            ));
        }
        Ok(self)
    }
}

fn system_id_from_blob(value: Vec<u8>) -> Result<SystemId, rusqlite::Error> {
    let bytes: [u8; 16] = value.try_into().map_err(|bytes: Vec<u8>| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Blob,
            format!("system_id must be 16 bytes, got {}", bytes.len()).into(),
        )
    })?;
    Ok(SystemId::from_bytes(bytes))
}

pub fn device_auth_generation(conn: &Connection) -> Result<i64, OpsError> {
    Ok(conn.query_row(
        "SELECT device_credential_generation FROM auth_state WHERE id = 1",
        [],
        |row| row.get(0),
    )?)
}

pub(crate) fn register_device_principal_in_tx(
    tx: &Transaction<'_>,
    principal_id: &str,
    device_system_id: &SystemId,
    scopes: &[SystemId],
    flow_class: &str,
    now_ms: i64,
) -> Result<(), OpsError> {
    if scopes.is_empty() {
        return Err(OpsError::Validation(
            "at least one registered system_id scope is required".into(),
        ));
    }
    if scopes.len() > DEVICE_PRINCIPAL_SCOPE_CAP {
        return Err(OpsError::Validation(
            "principal_scope_limit_exceeded".into(),
        ));
    }
    let device_live: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM devices WHERE system_id = ?1 AND state != 'retired')",
        [device_system_id.as_bytes().as_slice()],
        |row| row.get(0),
    )?;
    if !device_live {
        return Err(OpsError::NotFound);
    }
    tx.execute(
        "INSERT INTO device_ingest_principals (
           principal_id, device_system_id, flow_class, profile, created_at
         ) VALUES (?1, ?2, ?3, 'simple_bearer', ?4)",
        params![
            principal_id,
            device_system_id.as_bytes().as_slice(),
            flow_class,
            now_ms
        ],
    )?;
    for scope in scopes {
        tx.execute(
            "INSERT INTO device_principal_scopes (principal_id, system_id) VALUES (?1, ?2)",
            params![principal_id, scope.as_bytes().as_slice()],
        )?;
    }
    iotkit_core_ledger::record_event(
        tx, "device_principal_authority", Some(device_system_id),
        &serde_json::json!({"code":"principal_registered","principal_id":principal_id,"scope_count":scopes.len(),"flow_class":flow_class}).to_string(),
    )?;
    Ok(())
}

pub(crate) fn issue_device_credential_in_tx(
    tx: &Transaction<'_>,
    principal_id: &str,
    state: DeviceCredentialState,
    reason: &str,
    entropy: &mut dyn CredentialEntropy,
    clock: &dyn CredentialClock,
) -> Result<(String, DeviceCredentialPresentation), OpsError> {
    if state == DeviceCredentialState::Revoked {
        return Err(OpsError::Validation(
            "cannot issue a revoked credential".into(),
        ));
    }
    let reason = CredentialReasonCode::parse(reason)?.issue_code()?;
    if state == DeviceCredentialState::Pending && reason != "credential_reissue" {
        return Err(OpsError::Validation(
            "pending issuance requires credential_reissue".into(),
        ));
    }
    if state == DeviceCredentialState::Current && reason == "credential_reissue" {
        return Err(OpsError::Validation(
            "initial issuance cannot use credential_reissue".into(),
        ));
    }
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM device_ingest_principals p
          JOIN devices d ON d.system_id=p.device_system_id
          WHERE p.principal_id=?1 AND d.state!='retired'
            AND EXISTS (
              SELECT 1 FROM device_principal_scopes s
              JOIN devices sd ON sd.system_id=s.system_id AND sd.state!='retired'
              WHERE s.principal_id=p.principal_id
            ))",
        [principal_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(OpsError::NotFound);
    }
    let auth_epoch = crate::auth_epoch(tx)?;
    let credential_id = random_identifier("dcr_", ID_BYTES, entropy)?;
    let plaintext = random_prefixed("ikd_", TOKEN_BYTES, entropy)?;
    let token_hash = Sha256::digest(plaintext.as_bytes());
    tx.execute(
        "INSERT INTO device_credentials (
           credential_id, principal_id, token_hash, auth_epoch, state, issued_at, issue_reason
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            credential_id,
            principal_id,
            token_hash.as_slice(),
            auth_epoch,
            state.as_str(),
            clock.now_ms(),
            reason
        ],
    )?;
    iotkit_core_ledger::record_event(
        tx, "device_credential_authority", None,
        &serde_json::json!({"code":"credential_issued","credential_id":credential_id,"principal_id":principal_id,"state":state.as_str(),"reason_code":reason}).to_string(),
    )?;
    Ok((credential_id, DeviceCredentialPresentation(plaintext)))
}

pub fn authenticate_device(
    conn: &Connection,
    plaintext: &str,
) -> Result<Option<DeviceAuthentication>, OpsError> {
    authenticate_device_with_clock(conn, plaintext, &SystemCredentialClock)
}

pub(crate) fn authenticate_device_with_clock(
    conn: &Connection,
    plaintext: &str,
    clock: &dyn CredentialClock,
) -> Result<Option<DeviceAuthentication>, OpsError> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let result = authenticate_device_in_tx(&tx, plaintext, clock)?;
    tx.commit()?;
    Ok(result)
}

fn authenticate_device_in_tx(
    tx: &Transaction<'_>,
    plaintext: &str,
    clock: &dyn CredentialClock,
) -> Result<Option<DeviceAuthentication>, OpsError> {
    let candidate = Sha256::digest(plaintext.as_bytes());
    let (current_epoch, auth_generation): (String, i64) = tx.query_row(
        "SELECT auth_epoch, auth_generation FROM auth_state WHERE id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let row = tx
        .query_row(
            "SELECT c.credential_id, c.principal_id, c.token_hash, c.auth_epoch, c.state,
                p.device_system_id, p.flow_class, p.profile, c.last_used_at,
                c.issued_at, c.proven_at, c.confirmed_at
         FROM device_credentials c
         JOIN live_device_ingest_principals p ON p.principal_id=c.principal_id
         WHERE c.token_hash=?1 AND c.auth_epoch=?2 AND c.state IN ('current','pending')
         LIMIT 1",
            params![candidate.as_slice(), current_epoch],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<i64>>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, Option<i64>>(10)?,
                    row.get::<_, Option<i64>>(11)?,
                ))
            },
        )
        .optional()?;
    let Some((
        credential_id,
        principal_id,
        stored_hash,
        epoch,
        state,
        device,
        flow,
        profile,
        last_used,
        issued_at,
        proven_at,
        confirmed_at,
    )) = row
    else {
        let zero = [0_u8; 32];
        let _ = candidate.as_slice().ct_eq(&zero).unwrap_u8();
        return Ok(None);
    };
    if stored_hash.len() != 32
        || candidate
            .as_slice()
            .ct_eq(stored_hash.as_slice())
            .unwrap_u8()
            != 1
    {
        return Ok(None);
    }
    let scopes = principal_scopes(tx, &principal_id)?;
    if scopes.is_empty() || scopes.len() > DEVICE_PRINCIPAL_SCOPE_CAP {
        return Ok(None);
    }
    let state = DeviceCredentialState::parse(&state)?;
    let observed_now = clock.now_ms();
    let now = logical_lifecycle_time(
        observed_now,
        [Some(issued_at), last_used, proven_at, confirmed_at],
    );
    let mut proven_changed = false;
    if state == DeviceCredentialState::Pending {
        let changed = tx.execute(
            "UPDATE device_credentials SET proven_at=?1
             WHERE credential_id=?2 AND state='pending' AND proven_at IS NULL",
            params![now, credential_id],
        )?;
        if changed == 1 {
            proven_changed = true;
        } else {
            let still_pending: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM device_credentials
                 WHERE credential_id=?1 AND state='pending' AND proven_at IS NOT NULL)",
                [&credential_id],
                |row| row.get(0),
            )?;
            if !still_pending {
                return Err(OpsError::Conflict);
            }
        }
    }
    if last_used
        .is_none_or(|last| observed_now.saturating_sub(last) >= LAST_USED_UPDATE_INTERVAL_MS)
    {
        let changed = tx.execute(
            "UPDATE device_credentials SET last_used_at=?1
             WHERE credential_id=?2 AND state IN ('current','pending') AND auth_epoch=?3",
            params![now, credential_id, current_epoch],
        )?;
        if changed != 1 {
            return Err(OpsError::Conflict);
        }
    }
    if proven_changed {
        iotkit_core_ledger::record_event(
            tx,
            "device_credential_use",
            None,
            &serde_json::json!({"code":"pending_credential_proven","credential_id":credential_id})
                .to_string(),
        )?;
    }
    let material_generation = device_auth_generation(tx)?;
    Ok(Some(DeviceAuthentication {
        principal: DevicePrincipal {
            principal_id,
            credential_id,
            device_system_id: system_id_from_blob(device)?,
            scopes,
            flow_class: flow,
            profile,
            auth_epoch: epoch,
        },
        credential_state: state,
        auth_generation,
        principal_material_generation: material_generation,
    }))
}

pub(crate) fn confirm_device_credential_in_tx(
    tx: &Transaction<'_>,
    principal_id: &str,
    credential_id: &str,
    reason: &str,
    now_ms: i64,
) -> Result<(), OpsError> {
    let reason = CredentialReasonCode::parse(reason)?.revoke_code()?;
    if reason != "credential_confirmed" {
        return Err(OpsError::Validation(
            "confirm requires credential_confirmed".into(),
        ));
    }
    let pending: Option<(i64, i64, Option<i64>)> = tx
        .query_row(
            "SELECT proven_at, issued_at, last_used_at FROM device_credentials
         WHERE credential_id=?1 AND principal_id=?2 AND state='pending'",
            params![credential_id, principal_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((proven_at, issued_at, last_used_at)) = pending else {
        return Err(OpsError::Conflict);
    };
    let now_ms = logical_lifecycle_time(
        now_ms,
        [Some(issued_at), Some(proven_at), last_used_at, None],
    );
    let principal_floor: i64 = tx.query_row(
        "SELECT COALESCE(MAX(MAX(
           issued_at,
           COALESCE(last_used_at, issued_at),
           COALESCE(proven_at, issued_at),
           COALESCE(confirmed_at, issued_at)
         )), ?2)
         FROM device_credentials
         WHERE principal_id=?1 AND state IN ('current','pending')",
        params![principal_id, now_ms],
        |row| row.get(0),
    )?;
    let now_ms = now_ms.max(principal_floor);
    tx.execute(
        "UPDATE device_credentials SET state='revoked', revoked_at=?1, revoke_reason=?2
         WHERE principal_id=?3 AND state='current'",
        params![now_ms, reason, principal_id],
    )?;
    let changed = tx.execute(
        "UPDATE device_credentials SET state='current', confirmed_at=?1
         WHERE credential_id=?2 AND principal_id=?3 AND state='pending' AND proven_at IS NOT NULL",
        params![now_ms, credential_id, principal_id],
    )?;
    if changed != 1 {
        return Err(OpsError::Conflict);
    }
    iotkit_core_ledger::record_event(
        tx, "device_credential_authority", None,
        &serde_json::json!({"code":"credential_confirmed","credential_id":credential_id,"principal_id":principal_id,"reason_code":reason}).to_string(),
    )?;
    Ok(())
}

pub(crate) fn abandon_device_credential_in_tx(
    tx: &Transaction<'_>,
    principal_id: &str,
    credential_id: &str,
    reason: &str,
    now_ms: i64,
) -> Result<(), OpsError> {
    revoke_matching_in_tx(
        tx,
        principal_id,
        credential_id,
        Some("pending"),
        reason,
        now_ms,
    )
}

pub(crate) fn revoke_device_credential_in_tx(
    tx: &Transaction<'_>,
    principal_id: &str,
    credential_id: &str,
    reason: &str,
    now_ms: i64,
) -> Result<(), OpsError> {
    revoke_matching_in_tx(tx, principal_id, credential_id, None, reason, now_ms)
}

fn revoke_matching_in_tx(
    tx: &Transaction<'_>,
    principal_id: &str,
    credential_id: &str,
    required_state: Option<&str>,
    reason: &str,
    now_ms: i64,
) -> Result<(), OpsError> {
    let reason = CredentialReasonCode::parse(reason)?.revoke_code()?;
    if required_state == Some("pending") && reason != "pending_abandoned" {
        return Err(OpsError::Validation(
            "abandon requires pending_abandoned".into(),
        ));
    }
    if required_state.is_none() && reason != "operator_revoked" {
        return Err(OpsError::Validation(
            "revoke requires operator_revoked".into(),
        ));
    }
    let lifecycle: Option<LifecycleTimes> = tx
        .query_row(
            "SELECT issued_at, last_used_at, proven_at, confirmed_at FROM device_credentials
             WHERE credential_id=?1 AND principal_id=?2 AND state!='revoked'",
            params![credential_id, principal_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((issued_at, last_used_at, proven_at, confirmed_at)) = lifecycle else {
        return Err(OpsError::Conflict);
    };
    let now_ms = logical_lifecycle_time(
        now_ms,
        [Some(issued_at), last_used_at, proven_at, confirmed_at],
    );
    let changed = match required_state {
        Some(state) => tx.execute(
            "UPDATE device_credentials SET state='revoked', revoked_at=?1, revoke_reason=?2
             WHERE credential_id=?3 AND principal_id=?4 AND state=?5",
            params![now_ms, reason, credential_id, principal_id, state],
        )?,
        None => tx.execute(
            "UPDATE device_credentials SET state='revoked', revoked_at=?1, revoke_reason=?2
             WHERE credential_id=?3 AND principal_id=?4 AND state!='revoked'",
            params![now_ms, reason, credential_id, principal_id],
        )?,
    };
    if changed != 1 {
        return Err(OpsError::Conflict);
    }
    iotkit_core_ledger::record_event(
        tx, "device_credential_authority", None,
        &serde_json::json!({"code":"credential_revoked","credential_id":credential_id,"principal_id":principal_id,"reason_code":reason}).to_string(),
    )?;
    recover_capacity_debt_if_possible_in_tx(tx, now_ms)?;
    Ok(())
}

fn logical_lifecycle_time<const N: usize>(observed: i64, stored: [Option<i64>; N]) -> i64 {
    stored.into_iter().flatten().fold(observed, i64::max)
}

pub fn list_device_credentials(conn: &Connection) -> Result<Vec<DeviceCredentialRow>, OpsError> {
    let mut stmt = conn.prepare(
        "SELECT credential_id, principal_id, state, issued_at, last_used_at, proven_at,
                confirmed_at, revoked_at, issue_reason, revoke_reason
         FROM device_credentials ORDER BY issued_at, credential_id",
    )?;
    let raw = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, Option<String>>(9)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    raw.into_iter()
        .map(|row| {
            Ok(DeviceCredentialRow {
                credential_id: row.0,
                principal_id: row.1,
                state: DeviceCredentialState::parse(&row.2)?,
                issued_at: row.3,
                last_used_at: row.4,
                proven_at: row.5,
                confirmed_at: row.6,
                revoked_at: row.7,
                issue_reason: row.8,
                revoke_reason: row.9,
            })
        })
        .collect()
}

pub fn list_device_principals(conn: &Connection) -> Result<Vec<DevicePrincipalRow>, OpsError> {
    let mut stmt = conn.prepare(
        "SELECT principal_id, device_system_id, flow_class, profile
         FROM device_ingest_principals ORDER BY principal_id",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|(principal_id, device, flow_class, profile)| {
            Ok(DevicePrincipalRow {
                scopes: principal_scopes(conn, &principal_id)?,
                principal_id,
                device_system_id: system_id_from_blob(device)?,
                flow_class,
                profile,
            })
        })
        .collect()
}

fn principal_scopes(conn: &Connection, principal_id: &str) -> Result<Vec<SystemId>, OpsError> {
    let mut stmt = conn.prepare(
        "SELECT s.system_id FROM device_principal_scopes s
         JOIN devices d ON d.system_id=s.system_id AND d.state!='retired'
         WHERE s.principal_id=?1 ORDER BY s.system_id LIMIT ?2",
    )?;
    let limit = i64::try_from(DEVICE_PRINCIPAL_SCOPE_CAP + 1)
        .map_err(|_| OpsError::Validation("principal_scope_limit_exceeded".into()))?;
    Ok(stmt
        .query_map(params![principal_id, limit], |row| {
            system_id_from_blob(row.get(0)?)
        })?
        .collect::<Result<Vec<_>, _>>()?)
}

fn live_class_counts(conn: &Connection) -> Result<(i64, i64, i64), OpsError> {
    Ok(conn.query_row(
        "SELECT
           COALESCE(SUM(CASE WHEN flow_class='low' THEN 1 ELSE 0 END),0),
           COALESCE(SUM(CASE WHEN flow_class='default' THEN 1 ELSE 0 END),0),
           COALESCE(SUM(CASE WHEN flow_class='high' THEN 1 ELSE 0 END),0)
         FROM live_device_ingest_principals",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?)
}

fn required_capacity(conn: &Connection) -> Result<(i64, i64), OpsError> {
    let (low, default, high) = live_class_counts(conn)?;
    let mut stmt = conn
        .prepare("SELECT steady_units, burst_units FROM device_flow_classes WHERE flow_class=?1")?;
    let mut steady = 0_i64;
    let mut burst = 0_i64;
    for (class, count) in [("low", low), ("default", default), ("high", high)] {
        let (class_steady, class_burst): (i64, i64) =
            stmt.query_row([class], |row| Ok((row.get(0)?, row.get(1)?)))?;
        steady = checked_capacity_add(steady, checked_capacity_mul(count, class_steady)?)?;
        burst = checked_capacity_add(burst, checked_capacity_mul(count, class_burst)?)?;
    }
    Ok((steady, burst))
}

pub fn capacity_status(
    conn: &Connection,
    replacement: Option<(&str, &str)>,
) -> Result<CapacityStatus, OpsError> {
    let (capacity_steady_units, capacity_burst_units) = conn.query_row(
        "SELECT steady_units, burst_units FROM device_capacity WHERE id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let (mut required_steady_units, mut required_burst_units) = required_capacity(conn)?;
    if let Some((principal_id, new_class)) = replacement {
        let (new_steady, new_burst): (i64, i64) = conn
            .query_row(
                "SELECT steady_units, burst_units FROM device_flow_classes WHERE flow_class=?1",
                [new_class],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| OpsError::Validation("unknown flow class".into()))?;
        let old: Option<(i64, i64, bool)> = conn
            .query_row(
                "SELECT f.steady_units, f.burst_units,
                       EXISTS(SELECT 1 FROM live_device_ingest_principals live
                              WHERE live.principal_id=p.principal_id)
             FROM device_ingest_principals p
             JOIN devices d ON d.system_id=p.device_system_id AND d.state!='retired'
             JOIN device_flow_classes f ON f.flow_class=p.flow_class
             WHERE p.principal_id=?1",
                [principal_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if old.is_none() {
            return Err(OpsError::NotFound);
        }
        if let Some((old_steady, old_burst, true)) = old {
            required_steady_units = checked_capacity_add(
                checked_capacity_sub(required_steady_units, old_steady)?,
                new_steady,
            )?;
            required_burst_units = checked_capacity_add(
                checked_capacity_sub(required_burst_units, old_burst)?,
                new_burst,
            )?;
        }
    }
    Ok(CapacityStatus {
        required_steady_units,
        required_burst_units,
        capacity_steady_units,
        capacity_burst_units,
    })
}

pub fn capacity_status_for_new(
    conn: &Connection,
    flow_class: &str,
) -> Result<CapacityStatus, OpsError> {
    let mut status = capacity_status(conn, None)?;
    let (steady, burst): (i64, i64) = conn
        .query_row(
            "SELECT steady_units, burst_units FROM device_flow_classes WHERE flow_class=?1",
            [flow_class],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| OpsError::Validation("unknown flow class".into()))?;
    status.required_steady_units = checked_capacity_add(status.required_steady_units, steady)?;
    status.required_burst_units = checked_capacity_add(status.required_burst_units, burst)?;
    Ok(status)
}

pub(crate) fn capacity_status_for_activation(
    conn: &Connection,
    principal_id: &str,
) -> Result<CapacityStatus, OpsError> {
    let mut status = capacity_status(conn, None)?;
    let row: Option<(i64, i64, bool)> = conn
        .query_row(
            "SELECT f.steady_units, f.burst_units,
                    EXISTS(SELECT 1 FROM live_device_ingest_principals live
                           WHERE live.principal_id=p.principal_id)
             FROM device_ingest_principals p
             JOIN devices d ON d.system_id=p.device_system_id AND d.state!='retired'
             JOIN device_flow_classes f ON f.flow_class=p.flow_class
             WHERE p.principal_id=?1
               AND EXISTS (
                 SELECT 1 FROM device_principal_scopes s
                 JOIN devices sd ON sd.system_id=s.system_id AND sd.state!='retired'
                 WHERE s.principal_id=p.principal_id
               )",
            [principal_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((steady, burst, already_live)) = row else {
        return Err(OpsError::NotFound);
    };
    if !already_live {
        status.required_steady_units = checked_capacity_add(status.required_steady_units, steady)?;
        status.required_burst_units = checked_capacity_add(status.required_burst_units, burst)?;
    }
    Ok(status)
}

fn active_debt_status(conn: &Connection) -> Result<Option<CapacityStatus>, OpsError> {
    Ok(conn.query_row(
        "SELECT required_steady_units, required_burst_units, capacity_steady_units, capacity_burst_units
         FROM capacity_debt WHERE recovered_at IS NULL",
        [], |row| Ok(CapacityStatus { required_steady_units: row.get(0)?,
            required_burst_units: row.get(1)?, capacity_steady_units: row.get(2)?,
            capacity_burst_units: row.get(3)? }),
    ).optional()?)
}

pub(crate) fn capacity_change_requires_approval(
    conn: &Connection,
    prospective: CapacityStatus,
) -> Result<bool, OpsError> {
    if !prospective.exceeds() {
        return Ok(false);
    }
    Ok(match active_debt_status(conn)? {
        Some(approved) => !prospective.no_worse_than(approved)?,
        None => true,
    })
}

fn record_capacity_event(
    tx: &Transaction<'_>,
    code: &str,
    status: CapacityStatus,
    now_ms: i64,
) -> Result<(), OpsError> {
    let detail = serde_json::json!({
        "code": code, "at": now_ms,
        "required_steady_units": status.required_steady_units,
        "required_burst_units": status.required_burst_units,
        "capacity_steady_units": status.capacity_steady_units,
        "capacity_burst_units": status.capacity_burst_units,
    });
    iotkit_core_ledger::record_event(tx, "capacity_debt", None, &detail.to_string())?;
    Ok(())
}

pub(crate) fn approve_capacity_debt_in_tx(
    tx: &Transaction<'_>,
    status: CapacityStatus,
    actor: &str,
    operation: &str,
    now_ms: i64,
) -> Result<(), OpsError> {
    if !matches!(
        operation,
        "device_add" | "credential_issue" | "flow_class_change" | "authority_configure"
    ) {
        return Err(OpsError::Validation(
            "unknown capacity debt operation code".into(),
        ));
    }
    if !status.exceeds() {
        return Ok(());
    }
    if active_debt_status(tx)?.is_some() {
        tx.execute(
            "UPDATE capacity_debt SET approved_at=?1, changed_at=?1, approved_by=?2, operation=?3,
             required_steady_units=?4, required_burst_units=?5,
             capacity_steady_units=?6, capacity_burst_units=?7 WHERE recovered_at IS NULL",
            params![
                now_ms,
                actor,
                operation,
                status.required_steady_units,
                status.required_burst_units,
                status.capacity_steady_units,
                status.capacity_burst_units
            ],
        )?;
        record_capacity_event(tx, "capacity_debt_changed", status, now_ms)?;
    } else {
        tx.execute(
            "INSERT INTO capacity_debt (
               approved_at, changed_at, approved_by, operation, required_steady_units,
               required_burst_units, capacity_steady_units, capacity_burst_units
             ) VALUES (?1,?1,?2,?3,?4,?5,?6,?7)",
            params![
                now_ms,
                actor,
                operation,
                status.required_steady_units,
                status.required_burst_units,
                status.capacity_steady_units,
                status.capacity_burst_units
            ],
        )?;
        record_capacity_event(tx, "capacity_debt_created", status, now_ms)?;
    }
    Ok(())
}

pub(crate) fn change_device_flow_class_in_tx(
    tx: &Transaction<'_>,
    principal_id: &str,
    flow_class: &str,
    approve_debt: bool,
    actor: &str,
    now_ms: i64,
) -> Result<CapacityStatus, OpsError> {
    let status = capacity_status(tx, Some((principal_id, flow_class)))?;
    let active = active_debt_status(tx)?;
    if status.exceeds() && !approve_debt {
        let reduction = match active {
            Some(approved) => status.no_worse_than(approved)?,
            None => false,
        };
        if !reduction {
            return Err(OpsError::Conflict);
        }
    }
    let changed = tx.execute(
        "UPDATE device_ingest_principals SET flow_class=?1
         WHERE principal_id=?2 AND EXISTS(
           SELECT 1 FROM devices d WHERE d.system_id=device_system_id AND d.state!='retired'
         )",
        params![flow_class, principal_id],
    )?;
    if changed != 1 {
        return Err(OpsError::NotFound);
    }
    iotkit_core_ledger::record_event(
        tx, "device_principal_authority", None,
        &serde_json::json!({"code":"flow_class_changed","principal_id":principal_id,"flow_class":flow_class}).to_string(),
    )?;
    if status.exceeds() {
        if approve_debt {
            approve_capacity_debt_in_tx(tx, status, actor, "flow_class_change", now_ms)?;
        } else {
            tx.execute(
                "UPDATE capacity_debt SET changed_at=?1, required_steady_units=?2,
                 required_burst_units=?3, capacity_steady_units=?4, capacity_burst_units=?5
                 WHERE recovered_at IS NULL",
                params![
                    now_ms,
                    status.required_steady_units,
                    status.required_burst_units,
                    status.capacity_steady_units,
                    status.capacity_burst_units
                ],
            )?;
            record_capacity_event(tx, "capacity_debt_changed", status, now_ms)?;
        }
    } else {
        recover_capacity_debt_if_possible_in_tx(tx, now_ms)?;
    }
    Ok(status)
}

pub(crate) fn recover_capacity_debt_if_possible_in_tx(
    tx: &Transaction<'_>,
    now_ms: i64,
) -> Result<bool, OpsError> {
    let status = capacity_status(tx, None)?;
    if status.exceeds() {
        return Ok(false);
    }
    let changed = tx.execute(
        "UPDATE capacity_debt SET recovered_at=?1, changed_at=?1 WHERE recovered_at IS NULL",
        [now_ms],
    )?;
    if changed > 0 {
        record_capacity_event(tx, "capacity_debt_recovered", status, now_ms)?;
    }
    Ok(changed > 0)
}

pub fn capacity_health(conn: &Connection) -> Result<CapacityHealth, OpsError> {
    Ok(CapacityHealth {
        status: capacity_status(conn, None)?,
        active_debt: active_debt_status(conn)?.is_some(),
    })
}

pub fn configured_stale_after_ms(conn: &Connection) -> Result<i64, OpsError> {
    Ok(conn.query_row(
        "SELECT stale_after_ms FROM device_capacity WHERE id=1",
        [],
        |row| row.get(0),
    )?)
}

pub fn stale_credential_health(
    conn: &Connection,
    now_ms: i64,
    stale_after_ms: i64,
) -> Result<StaleCredentialHealth, OpsError> {
    if stale_after_ms <= 0 {
        return Err(OpsError::Validation(
            "stale threshold must be positive".into(),
        ));
    }
    let cutoff = now_ms.saturating_sub(stale_after_ms);
    let mut stmt = conn.prepare(
        "SELECT COALESCE(c.last_used_at, c.issued_at) FROM device_credentials c
         JOIN live_device_ingest_principals p ON p.principal_id=c.principal_id
         WHERE c.state IN ('current','pending') ORDER BY c.credential_id LIMIT ?1",
    )?;
    let values = stmt
        .query_map([i64::try_from(HEALTH_COUNT_CAP + 1).unwrap()], |row| {
            row.get::<_, i64>(0)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let capped = values.len() > HEALTH_COUNT_CAP;
    let visible = &values[..values.len().min(HEALTH_COUNT_CAP)];
    Ok(StaleCredentialHealth {
        active_count: visible.len() as u64,
        stale_count: visible.iter().filter(|value| **value < cutoff).count() as u64,
        counts_capped: capped,
    })
}

pub fn replacement_backup_health(conn: &Connection) -> Result<ReplacementBackupHealth, OpsError> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM device_credentials)",
        [],
        |row| row.get(0),
    )?;
    Ok(ReplacementBackupHealth {
        replacement_backup_unavailable: exists,
        recovery_action: exists.then_some(REPLACEMENT_BACKUP_ACTION),
    })
}

pub(crate) fn configure_device_authority_in_tx(
    tx: &Transaction<'_>,
    config: DeviceAuthorityConfig,
    approve_debt: bool,
    actor: &str,
    now_ms: i64,
) -> Result<(), OpsError> {
    let config = config.validate()?;
    for (name, weight) in [
        ("low", config.low),
        ("default", config.default),
        ("high", config.high),
    ] {
        tx.execute(
            "UPDATE device_flow_classes SET steady_units=?1, burst_units=?2 WHERE flow_class=?3",
            params![weight.steady_units, weight.burst_units, name],
        )?;
    }
    tx.execute(
        "UPDATE device_capacity SET steady_units=?1, burst_units=?2, stale_after_ms=?3 WHERE id=1",
        params![
            config.capacity.steady_units,
            config.capacity.burst_units,
            config.stale_after_ms
        ],
    )?;
    let status = capacity_status(tx, None)?;
    if status.exceeds() {
        let previous = active_debt_status(tx)?;
        if approve_debt {
            approve_capacity_debt_in_tx(tx, status, actor, "authority_configure", now_ms)?;
        } else {
            let Some(previous) = previous else {
                return Err(OpsError::Conflict);
            };
            if !status.no_worse_than(previous)? {
                return Err(OpsError::Conflict);
            }
            tx.execute(
                "UPDATE capacity_debt SET changed_at=?1, required_steady_units=?2,
                 required_burst_units=?3, capacity_steady_units=?4, capacity_burst_units=?5
                 WHERE recovered_at IS NULL",
                params![
                    now_ms,
                    status.required_steady_units,
                    status.required_burst_units,
                    status.capacity_steady_units,
                    status.capacity_burst_units
                ],
            )?;
            record_capacity_event(tx, "capacity_debt_changed", status, now_ms)?;
        }
    } else {
        recover_capacity_debt_if_possible_in_tx(tx, now_ms)?;
    }
    tx.execute(
        "UPDATE auth_state SET device_credential_generation=device_credential_generation+1 WHERE id=1",
        [],
    )?;
    iotkit_core_ledger::record_event(
        tx,
        "device_authority_config",
        None,
        &serde_json::json!({"code":"authority_configured"}).to_string(),
    )?;
    Ok(())
}

fn random_prefixed(
    prefix: &str,
    bytes: usize,
    entropy: &mut dyn CredentialEntropy,
) -> Result<Zeroizing<String>, OpsError> {
    let mut value = Zeroizing::new(vec![0_u8; bytes]);
    entropy.fill(value.as_mut_slice())?;
    let mut encoded = Zeroizing::new(String::with_capacity(prefix.len() + bytes.div_ceil(3) * 4));
    encoded.push_str(prefix);
    URL_SAFE_NO_PAD.encode_string(value.as_slice(), &mut encoded);
    Ok(encoded)
}

fn random_identifier(
    prefix: &str,
    bytes: usize,
    entropy: &mut dyn CredentialEntropy,
) -> Result<String, OpsError> {
    let mut value = Zeroizing::new(vec![0_u8; bytes]);
    entropy.fill(value.as_mut_slice())?;
    Ok(format!(
        "{prefix}{}",
        URL_SAFE_NO_PAD.encode(value.as_slice())
    ))
}
