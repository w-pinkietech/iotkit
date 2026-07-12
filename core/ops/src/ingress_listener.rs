use std::collections::BTreeSet;
use std::net::IpAddr;
use std::path::Path;

use ipnet::IpNet;
use rusqlite::Connection;
use rusqlite::{Transaction, TransactionBehavior, params};

use crate::OpsError;

/// Deliberately compiled false through Task 5. It is not persisted and has no override path.
pub const INGRESS_READY: bool = false;

pub(crate) fn stage_ingress_tls_generation(
    data_dir: &Path,
    generation: i64,
    cert: &[u8],
    key: &[u8],
) -> Result<(), crate::OpError> {
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    let root = data_dir.join("ingress-tls");
    std::fs::create_dir_all(&root).map_err(|_| tls_custody_error())?;
    #[cfg(unix)]
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
        .map_err(|_| tls_custody_error())?;
    let staging = root.join(format!(".generation-{generation}.staging"));
    remove_dir_if_present(&staging).map_err(|_| tls_custody_error())?;

    let staged = (|| -> Result<(), std::io::Error> {
        std::fs::create_dir(&staging)?;
        #[cfg(unix)]
        std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o700))?;
        for (name, bytes, mode) in [("cert.pem", cert, 0o644), ("key.pem", key, 0o600)] {
            let path = staging.join(name);
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(bytes)?;
            #[cfg(unix)]
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))?;
            file.sync_all()?;
        }
        std::fs::File::open(&staging)?.sync_all()?;
        std::fs::File::open(&root)?.sync_all()?;
        Ok(())
    })();
    if staged.is_err() {
        let _ = remove_dir_if_present(&staging);
        let _ = sync_dir(&root);
        return Err(tls_custody_error());
    }
    Ok(())
}

/// Reconciles filesystem custody against committed R14 state. Staging directories are never
/// authoritative: a referenced one is promoted only after its SQLite settlement is visible;
/// unreferenced staged/final generations are removed. The currently desired and last applied
/// generations are both retained so a failed switchover cannot destroy the last-safe key.
pub fn reconcile_ingress_tls_custody(conn: &Connection, data_dir: &Path) -> Result<(), OpsError> {
    let root = data_dir.join("ingress-tls");
    if !root.exists() {
        return Ok(());
    }
    let mut referenced = BTreeSet::new();
    let mut stmt = conn.prepare(
        "SELECT generation FROM ingress_tls_material
         UNION SELECT desired_tls_generation FROM ingress_listener_config
           WHERE desired_tls_generation IS NOT NULL
         UNION SELECT applied_tls_generation FROM ingress_listener_config
           WHERE applied_tls_generation IS NOT NULL",
    )?;
    for generation in stmt.query_map([], |row| row.get::<_, i64>(0))? {
        let generation = generation?;
        if generation <= 0 {
            return Err(OpsError::Validation(
                "unsafe_ingress_generation_state".into(),
            ));
        }
        referenced.insert(generation);
    }

    for generation in &referenced {
        let final_dir = root.join(format!("generation-{generation}"));
        let staging = root.join(format!(".generation-{generation}.staging"));
        if final_dir.is_dir() {
            remove_dir_if_present(&staging)?;
        } else if staging.is_dir() {
            std::fs::rename(&staging, &final_dir)?;
            sync_dir(&root)?;
        }
    }

    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let generation = parse_generation_dir(name);
        if generation.is_some_and(|generation| !referenced.contains(&generation)) {
            remove_dir_if_present(&entry.path())?;
        }
    }
    sync_dir(&root)?;
    Ok(())
}

fn parse_generation_dir(name: &str) -> Option<i64> {
    name.strip_prefix("generation-")
        .or_else(|| {
            name.strip_prefix(".generation-")
                .and_then(|name| name.strip_suffix(".staging"))
        })?
        .parse()
        .ok()
}

fn remove_dir_if_present(path: &Path) -> Result<(), std::io::Error> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn sync_dir(path: &Path) -> Result<(), std::io::Error> {
    std::fs::File::open(path)?.sync_all()
}

fn tls_custody_error() -> crate::OpError {
    crate::OpError::Internal("tls_custody_io".into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngressListenerMode {
    Tls,
    PrivatePlaintext,
}

impl IngressListenerMode {
    pub(crate) fn parse(value: &str) -> Result<Self, OpsError> {
        match value {
            "tls" => Ok(Self::Tls),
            "private_plaintext" => Ok(Self::PrivatePlaintext),
            _ => Err(OpsError::Validation("invalid_ingress_mode".into())),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Tls => "tls",
            Self::PrivatePlaintext => "private_plaintext",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngressListenerState {
    pub generation: u64,
    pub bind_addr: String,
    pub interface: String,
    pub site_local_cidrs: Vec<String>,
    pub mode: IngressListenerMode,
    pub tls_generation: Option<u64>,
    pub tls_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngressListenerConfig {
    pub enabled: bool,
    pub desired: IngressListenerState,
    pub applied: Option<IngressListenerState>,
    pub last_error: Option<String>,
    pub last_action: String,
}

pub fn load_ingress_listener_config(conn: &Connection) -> Result<IngressListenerConfig, OpsError> {
    conn.query_row(
        "SELECT desired_generation, applied_generation, enabled, bind_addr, interface,
                site_local_cidrs, mode, desired_tls_generation, desired_tls_fingerprint,
                applied_bind_addr, applied_interface, applied_site_local_cidrs, applied_mode,
                applied_tls_generation, applied_tls_fingerprint, last_error, last_action
         FROM ingress_listener_config WHERE id=1",
        [],
        |row| {
            let desired_cidrs: String = row.get(5)?;
            let applied_generation: i64 = row.get(1)?;
            let applied_cidrs: Option<String> = row.get(11)?;
            let applied_mode: Option<String> = row.get(12)?;
            Ok((
                row.get::<_, i64>(0)?,
                applied_generation,
                row.get::<_, bool>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                desired_cidrs,
                row.get::<_, String>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
                applied_cidrs,
                applied_mode,
                row.get::<_, Option<i64>>(13)?,
                row.get::<_, Option<String>>(14)?,
                row.get::<_, Option<String>>(15)?,
                row.get::<_, String>(16)?,
            ))
        },
    )
    .map_err(OpsError::from)
    .and_then(|row| {
        let desired = IngressListenerState {
            generation: checked_generation(row.0)?,
            bind_addr: row.3,
            interface: row.4,
            site_local_cidrs: parse_cidrs_json(&row.5)?,
            mode: IngressListenerMode::parse(&row.6)?,
            tls_generation: row.7.map(checked_generation).transpose()?,
            tls_fingerprint: row.8,
        };
        let applied = if row.1 == 0 {
            None
        } else {
            Some(IngressListenerState {
                generation: checked_generation(row.1)?,
                bind_addr: row
                    .9
                    .ok_or_else(|| OpsError::Validation("partial_applied_ingress_state".into()))?,
                interface: row
                    .10
                    .ok_or_else(|| OpsError::Validation("partial_applied_ingress_state".into()))?,
                site_local_cidrs: parse_cidrs_json(&row.11.ok_or_else(|| {
                    OpsError::Validation("partial_applied_ingress_state".into())
                })?)?,
                mode: IngressListenerMode::parse(&row.12.ok_or_else(|| {
                    OpsError::Validation("partial_applied_ingress_state".into())
                })?)?,
                tls_generation: row.13.map(checked_generation).transpose()?,
                tls_fingerprint: row.14,
            })
        };
        if applied
            .as_ref()
            .is_some_and(|state| state.generation > desired.generation)
        {
            return Err(OpsError::Validation(
                "unsafe_ingress_generation_state".into(),
            ));
        }
        if !stable_code(&row.16) || row.15.as_deref().is_some_and(|code| !stable_code(code)) {
            return Err(OpsError::Validation("corrupt_ingress_status".into()));
        }
        Ok(IngressListenerConfig {
            enabled: row.2,
            desired,
            applied,
            last_error: row.15,
            last_action: row.16,
        })
    })
}

/// R15 applied-state report. This is not a configuration bypass: it can only acknowledge the
/// exact currently committed desired generation and copies that committed state verbatim.
pub fn mark_ingress_applied(
    conn: &Connection,
    generation: u64,
    installed_tls_generation: Option<u64>,
) -> Result<(), OpsError> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    mark_ingress_applied_in_transaction(&tx, generation, installed_tls_generation)?;
    tx.commit()?;
    Ok(())
}

/// Transaction-held variant used when the runtime must couple authority revalidation and
/// applied-state publication under the same SQLite write lock.
pub fn mark_ingress_applied_in_transaction(
    tx: &Transaction<'_>,
    generation: u64,
    installed_tls_generation: Option<u64>,
) -> Result<(), OpsError> {
    let generation = i64::try_from(generation)
        .map_err(|_| OpsError::Validation("unsafe_ingress_generation_state".into()))?;
    let desired: (i64, bool, String, Option<i64>) = tx.query_row(
        "SELECT desired_generation,enabled,mode,desired_tls_generation
         FROM ingress_listener_config WHERE id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    if desired.0 != generation {
        return Err(OpsError::Conflict);
    }
    let installed_tls_generation = installed_tls_generation
        .map(i64::try_from)
        .transpose()
        .map_err(|_| OpsError::Validation("unsafe_ingress_generation_state".into()))?;
    let expected_tls = if desired.1 && desired.2 == "tls" {
        desired.3
    } else {
        None
    };
    if installed_tls_generation != expected_tls {
        return Err(OpsError::Validation("tls_generation_not_installed".into()));
    }
    tx.execute(
        "UPDATE ingress_listener_config SET applied_generation=desired_generation,
          applied_bind_addr=bind_addr, applied_interface=interface,
          applied_site_local_cidrs=site_local_cidrs, applied_mode=mode,
          applied_tls_generation=CASE WHEN ?2 IS NULL THEN NULL ELSE desired_tls_generation END,
          applied_tls_fingerprint=CASE WHEN ?2 IS NULL THEN NULL ELSE desired_tls_fingerprint END,
          last_error=NULL, last_action=CASE WHEN enabled THEN 'listening' ELSE 'disabled' END
         WHERE id=1 AND desired_generation=?1",
        params![generation, installed_tls_generation],
    )?;
    Ok(())
}

/// Runtime truth on process start, listener exit, or authority/inventory invalidation.
pub fn mark_ingress_runtime_unbound(conn: &Connection, action: &str) -> Result<(), OpsError> {
    if !stable_code(action) {
        return Err(OpsError::Validation("invalid_ingress_error_code".into()));
    }
    conn.execute(
        "UPDATE ingress_listener_config SET applied_generation=0,
         applied_bind_addr=NULL,applied_interface=NULL,applied_site_local_cidrs=NULL,
         applied_mode=NULL,applied_tls_generation=NULL,applied_tls_fingerprint=NULL,
         last_action=?1 WHERE id=1",
        [action],
    )?;
    Ok(())
}

/// Records a stable, payload-free action code while retaining the previous safe applied state.
pub fn mark_ingress_apply_error(
    conn: &Connection,
    desired_generation: u64,
    error_code: &str,
) -> Result<(), OpsError> {
    if error_code.is_empty()
        || error_code.len() > 64
        || !error_code
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(OpsError::Validation("invalid_ingress_error_code".into()));
    }
    let generation = i64::try_from(desired_generation)
        .map_err(|_| OpsError::Validation("unsafe_ingress_generation_state".into()))?;
    let changed = conn.execute(
        "UPDATE ingress_listener_config SET last_error=?1,last_action='apply_failed'
         WHERE id=1 AND desired_generation=?2",
        params![error_code, generation],
    )?;
    if changed != 1 {
        return Err(OpsError::Conflict);
    }
    Ok(())
}

pub(crate) fn validate_site_config(
    bind_addr: &str,
    interface: &str,
    cidrs: &[String],
    mode: IngressListenerMode,
) -> Result<(), crate::OpError> {
    if interface.len() > 64
        || cidrs.len() > 8
        || cidrs.iter().any(|cidr| cidr.is_empty() || cidr.len() > 64)
    {
        return Err(crate::OpError::Validation(
            "invalid_ingress_exposure_shape".into(),
        ));
    }
    let bind: std::net::SocketAddr = bind_addr
        .parse()
        .map_err(|_| crate::OpError::Validation("invalid_bind".into()))?;
    let ip = bind.ip().to_canonical();
    if ip.is_unspecified() || ip.is_loopback() || interface.trim().is_empty() || interface == "lo" {
        return Err(crate::OpError::Validation("unsafe_ingress_exposure".into()));
    }
    let private = match ip {
        IpAddr::V4(ip) => ip.is_private(),
        IpAddr::V6(ip) => (ip.segments()[0] & 0xfe00) == 0xfc00,
    };
    if !private || cidrs.is_empty() {
        return Err(crate::OpError::Validation("unsafe_ingress_exposure".into()));
    }
    let parsed = cidrs
        .iter()
        .map(|value| {
            value
                .parse::<IpNet>()
                .map_err(|_| crate::OpError::Validation("invalid_site_local_cidr".into()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if parsed.iter().any(|cidr| !private_network(cidr))
        || !parsed.iter().any(|cidr| cidr.contains(&ip))
    {
        return Err(crate::OpError::Validation("unsafe_ingress_exposure".into()));
    }
    if mode == IngressListenerMode::PrivatePlaintext && !private {
        return Err(crate::OpError::Validation("unsafe_plaintext".into()));
    }
    Ok(())
}

fn private_network(cidr: &IpNet) -> bool {
    match cidr {
        IpNet::V4(net) => [
            ipnet::Ipv4Net::new("10.0.0.0".parse().expect("literal"), 8).expect("literal"),
            ipnet::Ipv4Net::new("172.16.0.0".parse().expect("literal"), 12).expect("literal"),
            ipnet::Ipv4Net::new("192.168.0.0".parse().expect("literal"), 16).expect("literal"),
        ]
        .iter()
        .any(|block| block.contains(&net.network()) && block.contains(&net.broadcast())),
        IpNet::V6(net) => net.prefix_len() >= 7 && (net.network().segments()[0] & 0xfe00) == 0xfc00,
    }
}

fn parse_cidrs_json(value: &str) -> Result<Vec<String>, OpsError> {
    serde_json::from_str(value).map_err(|_| OpsError::Validation("corrupt_ingress_cidrs".into()))
}

fn checked_generation(value: i64) -> Result<u64, OpsError> {
    u64::try_from(value).map_err(|_| OpsError::Validation("unsafe_ingress_generation_state".into()))
}

fn stable_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}
