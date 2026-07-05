use clap::{Args, Subcommand};
use iotkit_core_ledger as ledger;
use iotkit_core_publish::store::{
    TargetRow, any_unacked_for_target, target_count, target_delete, target_get, target_insert,
    target_set_archive_responsible, target_set_token,
};
use rusqlite::Connection;
use std::time::{SystemTime, UNIX_EPOCH};

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

const TARGET_ID: &str = "archive";

#[derive(Subcommand)]
pub enum TargetCommand {
    Add(AddArgs),
    List,
    RotateToken(RotateArgs),
    Remove(RemoveArgs),
}

#[derive(Args)]
pub struct AddArgs {
    pub endpoint: String,
    pub token: String,
    #[arg(long, default_value_t = 1)]
    pub schema_version: u32,
}

#[derive(Args)]
pub struct RotateArgs {
    pub token: String,
}

#[derive(Args)]
pub struct RemoveArgs {
    #[arg(long)]
    pub abandon_custody: bool,
}

pub fn run_target_add(
    conn: &Connection,
    endpoint: &str,
    token: &str,
    schema_version: u32,
    smoke: &dyn Fn(&str, &str) -> Result<(), String>,
) -> AppResult<()> {
    if !endpoint.starts_with("https://") {
        return Err("target endpoint must use https://".into());
    }
    if schema_version != 1 {
        return Err("target schema_version must be 1".into());
    }
    if target_count(conn)? > 0 {
        return Err("target already registered".into());
    }

    let now = now_ms();
    crate::cmd::devices::mutate(conn, |tx| {
        target_insert(
            tx,
            &TargetRow {
                target_id: TARGET_ID.into(),
                endpoint_url: endpoint.into(),
                credential_token: token.into(),
                archive_responsible: false,
                schema_version: i64::from(schema_version),
                cursor_epoch: None,
                cursor_pub_seq: 0,
            },
            now,
        )?;
        let detail = serde_json::json!({
            "target_id": TARGET_ID,
            "endpoint": endpoint,
            "schema_version": schema_version,
        })
        .to_string();
        ledger::record_event(tx, "target_added", None, &detail)?;
        Ok(())
    })?;

    smoke(endpoint, token).map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;

    crate::cmd::devices::mutate(conn, |tx| {
        target_set_archive_responsible(tx, TARGET_ID, true)?;
        ledger::record_event(tx, "target_smoke_ok", None, TARGET_ID)?;
        Ok(())
    })?;

    Ok(())
}

pub fn run_target_rotate_token(
    conn: &Connection,
    new_token: &str,
    smoke: &dyn Fn(&str, &str) -> Result<(), String>,
) -> AppResult<()> {
    let target = target_get(conn)?.ok_or("no target")?;
    if !target.endpoint_url.starts_with("https://") {
        return Err("refusing to rotate token for a non-HTTPS endpoint".into());
    }
    let old_token = target.credential_token.clone();

    crate::cmd::devices::mutate(conn, |tx| {
        target_set_token(tx, &target.target_id, new_token)?;
        Ok(())
    })?;

    if let Err(e) = smoke(&target.endpoint_url, new_token) {
        crate::cmd::devices::mutate(conn, |tx| {
            target_set_token(tx, &target.target_id, &old_token)?;
            Ok(())
        })?;
        return Err(e.into());
    }

    Ok(())
}

pub fn run_target_remove(conn: &Connection, abandon_custody: bool) -> AppResult<()> {
    crate::cmd::devices::mutate(conn, |tx| {
        let current_epoch = ledger::ledger_epoch(tx)?;
        let target = target_get(tx)?.ok_or("no target")?;
        if !abandon_custody && any_unacked_for_target(tx, &current_epoch, &target)? {
            return Err("unacked custody rows remain; use --abandon-custody to force".into());
        }
        target_delete(tx, &target.target_id)?;
        let detail = serde_json::json!({
            "target_id": target.target_id,
            "abandon_custody": abandon_custody,
        })
        .to_string();
        let kind = if abandon_custody {
            "target_removed_abandon_custody"
        } else {
            "target_removed"
        };
        ledger::record_event(tx, kind, None, &detail)?;
        Ok(())
    })
}

pub fn run_target_list(conn: &Connection) -> AppResult<()> {
    let Some(target) = target_get(conn)? else {
        println!("no target");
        return Ok(());
    };

    println!("{}", format_target_line(&target));
    Ok(())
}

fn format_target_line(target: &TargetRow) -> String {
    format!(
        "{}\t{}\t***\tarchive_responsible={}\tschema_version={}\tcursor_epoch={}\tcursor_pub_seq={}",
        target.target_id,
        target.endpoint_url,
        target.archive_responsible,
        target.schema_version,
        target.cursor_epoch.as_deref().unwrap_or_default(),
        target.cursor_pub_seq
    )
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use iotkit_core_publish::store::{
        TargetRow, enqueue_measurement, target_advance_cursor, target_get, target_insert,
    };
    use iotkit_core_timeseries::{NewReading, insert_reading_v3};

    fn test_conn() -> Connection {
        let mut migrations = iotkit_core_storage::MIGRATIONS.to_vec();
        migrations.extend_from_slice(ledger::MIGRATIONS);
        migrations.extend_from_slice(iotkit_core_timeseries::MIGRATIONS);
        migrations.extend_from_slice(iotkit_core_registry::MIGRATIONS);
        migrations.extend_from_slice(iotkit_core_publish::MIGRATIONS);
        migrations.sort_by_key(|m| m.version);
        let conn = Connection::open_in_memory().unwrap();
        iotkit_core_storage::run_migrations(&conn, &migrations).unwrap();
        conn
    }

    fn ok_smoke(_: &str, _: &str) -> Result<(), String> {
        Ok(())
    }

    fn err_smoke(_: &str, _: &str) -> Result<(), String> {
        Err("smoke failed".into())
    }

    fn seed_target(conn: &Connection, token: &str) {
        target_insert(
            conn,
            &TargetRow {
                target_id: "archive".into(),
                endpoint_url: "https://archive.example/publish".into(),
                credential_token: token.into(),
                archive_responsible: true,
                schema_version: 1,
                cursor_epoch: None,
                cursor_pub_seq: 0,
            },
            1,
        )
        .unwrap();
    }

    fn seed_unacked_measurement(conn: &Connection) {
        let sid = ledger::insert_device(
            conn,
            &ledger::NewDevice {
                hardware_id: "ble:target-test".into(),
                user_label: None,
                parent: None,
                kind: ledger::DeviceKind::Individual,
                initial_state: ledger::DeviceState::Active,
            },
        )
        .unwrap();
        let series_id = ledger::ensure_series(
            conn,
            &sid,
            "temperature_c",
            ledger::CHANNEL_NA,
            ledger::DEFAULT_VARIANT,
            false,
            None,
        )
        .unwrap();
        let reading_seq = insert_reading_v3(
            conn,
            &NewReading {
                series_id,
                received_at_ms: 1_000,
                device_time_ms: None,
                time_source: "gateway".into(),
                values: vec![21.5],
                rssi: None,
                battery_pct: None,
                quarantined: false,
            },
        )
        .unwrap();
        let epoch = ledger::ledger_epoch(conn).unwrap();
        enqueue_measurement(conn, &epoch, reading_seq, 1_001).unwrap();
    }

    #[test]
    fn add_rejects_non_https() {
        let conn = test_conn();

        let result = run_target_add(
            &conn,
            "http://archive.example/publish",
            "token",
            1,
            &ok_smoke,
        );

        assert!(result.is_err());
        assert!(target_get(&conn).unwrap().is_none());
    }

    #[test]
    fn add_rejects_second_target() {
        let conn = test_conn();
        seed_target(&conn, "old-token");

        let result = run_target_add(
            &conn,
            "https://other.example/publish",
            "new-token",
            1,
            &ok_smoke,
        );

        assert!(result.is_err());
        let row = target_get(&conn).unwrap().unwrap();
        assert_eq!(row.endpoint_url, "https://archive.example/publish");
        assert_eq!(row.credential_token, "old-token");
    }

    #[test]
    fn add_keeps_archive_responsible_zero_until_smoke_ok() {
        let conn = test_conn();

        let result = run_target_add(
            &conn,
            "https://archive.example/publish",
            "token",
            1,
            &err_smoke,
        );

        assert!(result.is_err());
        let row = target_get(&conn).unwrap().unwrap();
        assert!(!row.archive_responsible);
        assert_eq!(row.credential_token, "token");
    }

    #[test]
    fn add_sets_archive_responsible_1_on_smoke_ok() {
        let conn = test_conn();

        run_target_add(
            &conn,
            "https://archive.example/publish",
            "token",
            1,
            &ok_smoke,
        )
        .unwrap();

        let row = target_get(&conn).unwrap().unwrap();
        assert!(row.archive_responsible);
        assert_eq!(row.schema_version, 1);
        assert_eq!(row.cursor_epoch, None);
        assert_eq!(row.cursor_pub_seq, 0);
    }

    #[test]
    fn list_masks_credential_token() {
        let target = TargetRow {
            target_id: "archive".into(),
            endpoint_url: "https://archive.example/publish".into(),
            credential_token: "real-secret-token".into(),
            archive_responsible: true,
            schema_version: 1,
            cursor_epoch: None,
            cursor_pub_seq: 0,
        };

        let line = format_target_line(&target);

        assert!(line.contains("***"));
        assert!(!line.contains("real-secret-token"));
    }

    #[test]
    fn add_rejects_schema_version_mismatch() {
        let conn = test_conn();

        let result = run_target_add(
            &conn,
            "https://archive.example/publish",
            "token",
            2,
            &ok_smoke,
        );

        assert!(result.is_err());
        assert!(target_get(&conn).unwrap().is_none());
    }

    #[test]
    fn remove_refuses_when_unacked_rows_exist_without_override() {
        let conn = test_conn();
        seed_target(&conn, "token");
        seed_unacked_measurement(&conn);

        let refused = run_target_remove(&conn, false);

        assert!(refused.is_err());
        assert!(target_get(&conn).unwrap().is_some());

        run_target_remove(&conn, true).unwrap();
        assert!(target_get(&conn).unwrap().is_none());
    }

    #[test]
    fn rotate_token_keeps_archive_responsible_1_and_cursor() {
        let conn = test_conn();
        seed_target(&conn, "old-token");
        let epoch = ledger::ledger_epoch(&conn).unwrap();
        target_advance_cursor(&conn, "archive", &epoch, 42).unwrap();

        run_target_rotate_token(&conn, "new-token", &ok_smoke).unwrap();

        let row = target_get(&conn).unwrap().unwrap();
        assert_eq!(row.credential_token, "new-token");
        assert!(row.archive_responsible);
        assert_eq!(row.cursor_epoch.as_deref(), Some(epoch.as_str()));
        assert_eq!(row.cursor_pub_seq, 42);
    }

    #[test]
    fn rotate_token_refuses_non_https_endpoint() {
        let conn = test_conn();
        target_insert(
            &conn,
            &TargetRow {
                target_id: "archive".into(),
                endpoint_url: "http://legacy.example".into(),
                credential_token: "old-token".into(),
                archive_responsible: true,
                schema_version: 1,
                cursor_epoch: None,
                cursor_pub_seq: 0,
            },
            1,
        )
        .unwrap();
        let smoke_called = std::cell::Cell::new(false);

        let result = run_target_rotate_token(&conn, "new-token", &|_, _| {
            smoke_called.set(true);
            Ok(())
        });

        assert!(result.is_err());
        assert!(!smoke_called.get());
        let row = target_get(&conn).unwrap().unwrap();
        assert_eq!(row.credential_token, "old-token");
    }

    #[test]
    fn rotate_token_smoke_failure_rolls_back_token_and_keeps_archive_responsible_1() {
        let conn = test_conn();
        seed_target(&conn, "old-token");

        let result = run_target_rotate_token(&conn, "new-token", &err_smoke);

        assert!(result.is_err());
        let row = target_get(&conn).unwrap().unwrap();
        assert_eq!(row.credential_token, "old-token");
        assert!(row.archive_responsible);
    }
}
