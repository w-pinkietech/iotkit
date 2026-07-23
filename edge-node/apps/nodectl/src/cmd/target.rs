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
    if iotkit_core_ops::ownership_state(conn)? != iotkit_core_ops::OwnershipState::Owned {
        return Err(
            "setupモード中は出口target登録不可（D13）。管理者パスフレーズを設定してから".into(),
        );
    }
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
#[path = "../../tests/unit/cmd/target_tests.rs"]
mod tests;
