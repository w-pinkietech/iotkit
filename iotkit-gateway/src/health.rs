use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct AdapterHealth {
    pub id: String,
    pub alive: bool,
    pub last_event_at: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
pub struct DbHealth {
    pub size_bytes: u64,
    pub disk_available_bytes: u64,
    pub watermark_exceeded: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct RetentionHealth {
    pub days: u64,
    pub last_purge_at: Option<i64>,
    pub last_purged_rows: u64,
}

#[derive(Debug, Clone)]
pub struct HealthState {
    pub started_at: Instant,
    pub collector_alive: bool,
    pub adapters: Vec<AdapterHealth>,
    pub db: DbHealth,
    pub retention: RetentionHealth,
}

impl HealthState {
    pub fn new(retention_days: u64) -> Self {
        Self {
            started_at: Instant::now(),
            collector_alive: true,
            adapters: Vec::new(),
            db: DbHealth {
                size_bytes: 0,
                disk_available_bytes: 0,
                watermark_exceeded: false,
            },
            retention: RetentionHealth {
                days: retention_days,
                last_purge_at: None,
                last_purged_rows: 0,
            },
        }
    }

    pub fn note_adapter_event(&mut self, id: &str, at_ms: i64) {
        match self.adapters.iter_mut().find(|a| a.id == id) {
            Some(adapter) => {
                adapter.alive = true;
                adapter.last_event_at = Some(at_ms);
            }
            None => self.adapters.push(AdapterHealth {
                id: id.to_string(),
                alive: true,
                last_event_at: Some(at_ms),
            }),
        }
    }

    pub fn note_adapter_closed(&mut self, id: &str) {
        match self.adapters.iter_mut().find(|a| a.id == id) {
            Some(adapter) => adapter.alive = false,
            None => self.adapters.push(AdapterHealth {
                id: id.to_string(),
                alive: false,
                last_event_at: None,
            }),
        }
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn write_health_json(path: &Path, epoch: &str, state: &HealthState) -> std::io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = temp_path(path);
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(render_health_json(epoch, state).as_bytes())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(tmp, path)?;
    Ok(())
}

pub fn spawn_health_writer(
    path: PathBuf,
    epoch: String,
    state: std::sync::Arc<std::sync::Mutex<HealthState>>,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let snapshot = state.lock().expect("health state mutex poisoned").clone();
            if let Err(e) = write_health_json(&path, &epoch, &snapshot) {
                tracing::error!(error = %e, path = %path.display(), "health json write failed");
            }
            tokio::time::sleep(interval).await;
        }
    })
}

fn temp_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.tmp", path.display()))
}

fn render_health_json(epoch: &str, state: &HealthState) -> String {
    let adapters = state
        .adapters
        .iter()
        .map(|adapter| {
            format!(
                r#"{{"id":"{}","alive":{},"last_event_at":{}}}"#,
                escape_json(&adapter.id),
                adapter.alive,
                opt_i64(adapter.last_event_at)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"schema":1,"written_at":{},"epoch":"{}","uptime_s":{},"collector_alive":{},"adapters":[{}],"db":{{"size_bytes":{},"disk_available_bytes":{},"watermark_exceeded":{}}},"retention":{{"days":{},"last_purge_at":{},"last_purged_rows":{}}}}}"#,
        now_ms(),
        escape_json(epoch),
        state.started_at.elapsed().as_secs(),
        state.collector_alive,
        adapters,
        state.db.size_bytes,
        state.db.disk_available_bytes,
        state.db.watermark_exceeded,
        state.retention.days,
        opt_i64(state.retention.last_purge_at),
        state.retention.last_purged_rows,
    )
}

fn opt_i64(v: Option<i64>) -> String {
    v.map(|n| n.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn escape_json(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn write_health_json_uses_temp_file_then_rename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("health.json");
        let tmp = dir.path().join("health.json.tmp");
        let state = HealthState {
            started_at: Instant::now() - Duration::from_secs(10),
            collector_alive: true,
            adapters: vec![AdapterHealth {
                id: "bravepi-mainboard".to_string(),
                alive: true,
                last_event_at: Some(1234),
            }],
            db: DbHealth {
                size_bytes: 42,
                disk_available_bytes: 1024,
                watermark_exceeded: false,
            },
            retention: RetentionHealth {
                days: 90,
                last_purge_at: Some(4567),
                last_purged_rows: 3,
            },
        };

        write_health_json(&path, "epoch-1", &state).unwrap();

        assert!(path.exists());
        assert!(!tmp.exists());
        let json = std::fs::read_to_string(path).unwrap();
        assert!(json.contains(r#""schema":1"#));
        assert!(json.contains(r#""epoch":"epoch-1""#));
        assert!(json.contains(r#""collector_alive":true"#));
        assert!(json.contains(r#""id":"bravepi-mainboard""#));
        assert!(json.contains(r#""size_bytes":42"#));
        assert!(json.contains(r#""days":90"#));
        assert!(json.contains(r#""uptime_s":10"#) || json.contains(r#""uptime_s":11"#));
    }
}
