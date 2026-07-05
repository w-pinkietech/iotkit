use clap::Args;
use iotkit_core_ledger as ledger;
use iotkit_core_registry as registry;
use iotkit_core_timeseries::{self, query as ts_query};
use iotkit_ingest_contract::ReadingItem;
use rusqlite::{Connection, params, params_from_iter};
use std::collections::BTreeSet;

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Args)]
pub struct ReplaceArgs {
    pub system_id_text: String,
    #[arg(long)]
    pub new_hardware_id: String,
    #[arg(long)]
    pub force: bool,
    #[arg(long)]
    pub yes: bool,
}

#[derive(Args)]
pub struct ReplaceUndoArgs {
    pub system_id_text: String,
    #[arg(long)]
    pub old_hardware_id: String,
    #[arg(long)]
    pub since: Option<i64>,
    #[arg(long)]
    pub abandon_custody: bool,
}

type Profile = BTreeSet<(String, i32)>;

#[derive(Debug)]
struct ReplaceEvent {
    at: i64,
    old_hw: String,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn series_profile(series: &[ledger::SeriesRow]) -> Profile {
    series
        .iter()
        .map(|row| (row.measurement_key.clone(), row.channel_index))
        .collect()
}

fn observed_channel(conn: &Connection, item: &ReadingItem) -> AppResult<i32> {
    let raw_channel = item
        .channel_index
        .map(i32::from)
        .unwrap_or(ledger::CHANNEL_NA);
    let channel_mode = match registry::get_entry(conn, &item.measurement_key)? {
        Some(entry) => Some(entry.channel_mode),
        None => registry::standard_catalog()
            .find(&item.measurement_key)
            .map(|entry| entry.channel_mode),
    };
    match channel_mode {
        Some(registry::ChannelMode::Single) => match item.channel_index {
            None | Some(0) => Ok(ledger::CHANNEL_NA),
            Some(_) => Ok(raw_channel),
        },
        _ => Ok(raw_channel),
    }
}

fn observed_profile(conn: &Connection, hardware_id: &str) -> AppResult<Profile> {
    let mut profile = Profile::new();
    let staged_limit = u32::try_from(iotkit_core_timeseries::STAGED_READINGS_CAP_PER_HW)
        .expect("STAGED_READINGS_CAP_PER_HW fits u32");
    for (received_at, payload_json) in
        ts_query::list_staged_for_hardware(conn, hardware_id, staged_limit)?
    {
        match serde_json::from_str::<ReadingItem>(&payload_json) {
            Ok(item) => {
                let channel = observed_channel(conn, &item)?;
                profile.insert((item.measurement_key, channel));
            }
            Err(e) => eprintln!(
                "warning: failed to deserialize staged reading for {hardware_id} at {received_at}: {e}"
            ),
        }
    }
    if let Some(candidate) = ledger::find_alive_by_hardware_id(conn, hardware_id)? {
        for row in ledger::list_series_for_device(conn, &candidate.system_id)? {
            profile.insert((row.measurement_key, row.channel_index));
        }
    }
    Ok(profile)
}

fn format_profile(profile: &Profile) -> String {
    if profile.is_empty() {
        return "(empty)".into();
    }
    profile
        .iter()
        .map(|(key, channel)| {
            if *channel == ledger::CHANNEL_NA {
                format!("{key}:na")
            } else {
                format!("{key}:{channel}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn check_observed_profile(
    conn: &Connection,
    system_id: &ledger::SystemId,
    new_hardware_id: &str,
) -> AppResult<()> {
    let target_series = ledger::list_series_for_device(conn, system_id)?;
    let target = series_profile(&target_series);
    let observed = observed_profile(conn, new_hardware_id)?;
    if observed.is_empty() {
        return Err(format!(
            "observed profile for {new_hardware_id} is empty; use --force to override"
        )
        .into());
    }
    if observed != target {
        return Err(format!(
            "observed profile mismatch for {new_hardware_id}; expected [{}], observed [{}]; use --force to override",
            format_profile(&target),
            format_profile(&observed)
        )
        .into());
    }
    Ok(())
}

fn confirm_replace(
    conn: &Connection,
    device: &ledger::DeviceRow,
    new_hardware_id: &str,
    series: &[ledger::SeriesRow],
) -> AppResult<()> {
    let label = device
        .user_label
        .clone()
        .unwrap_or_else(|| device.system_id.to_text());
    eprintln!("Replace device {} ({})", label, device.system_id.to_text());
    eprintln!("hardware: {} -> {new_hardware_id}", device.hardware_id);
    eprintln!("series: {}", series.len());
    for row in series.iter().take(5) {
        match ts_query::latest_by_series(conn, row.series_id)? {
            Some(reading) => eprintln!(
                "latest {}:{} event_time={} received_at={} values={}",
                row.measurement_key,
                if row.channel_index == ledger::CHANNEL_NA {
                    "na".into()
                } else {
                    row.channel_index.to_string()
                },
                reading.event_time,
                reading.received_at,
                serde_json::to_string(&reading.values)?
            ),
            None => eprintln!(
                "latest {}:{} (none)",
                row.measurement_key,
                if row.channel_index == ledger::CHANNEL_NA {
                    "na".into()
                } else {
                    row.channel_index.to_string()
                }
            ),
        }
    }
    eprintln!("type 'replace' to confirm:");
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    if line.trim() != "replace" {
        return Err("replace aborted".into());
    }
    Ok(())
}

pub fn run_replace(conn: &Connection, args: ReplaceArgs) -> AppResult<()> {
    let sid = ledger::SystemId::from_text(&args.system_id_text)?;
    let device = ledger::get_device(conn, &sid)?
        .filter(|row| row.state != ledger::DeviceState::Retired)
        .ok_or_else(|| format!("non-retired device {} not found", sid.to_text()))?;
    let series = ledger::list_series_for_device(conn, &sid)?;

    if !args.force {
        check_observed_profile(conn, &sid, &args.new_hardware_id)?;
    }
    if !args.yes {
        confirm_replace(conn, &device, &args.new_hardware_id, &series)?;
    }

    let outcome = super::devices::mutate(conn, |tx| {
        if !args.force {
            check_observed_profile(tx, &sid, &args.new_hardware_id)?;
        }
        Ok(ledger::replace_hardware(tx, &sid, &args.new_hardware_id)?)
    })?;
    println!(
        "{}\t{}\t{}",
        outcome.replaced.to_text(),
        outcome.old_hardware_id,
        outcome.retired_candidates.len()
    );
    Ok(())
}

fn latest_replace_event_for_current_hardware(
    conn: &Connection,
    system_id: &ledger::SystemId,
    current_hardware_id: &str,
) -> AppResult<ReplaceEvent> {
    let mut stmt = conn.prepare(
        "SELECT event_id, at, detail FROM ledger_events
         WHERE kind = 'hardware_replaced' AND system_id = ?1
         ORDER BY event_id DESC",
    )?;
    let mut rows = stmt.query(params![system_id.as_bytes().to_vec()])?;
    while let Some(row) = rows.next()? {
        let event_id: i64 = row.get(0)?;
        let event_at: i64 = row.get(1)?;
        let detail: String = row.get(2)?;
        let value: serde_json::Value = match serde_json::from_str(&detail) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if value.get("new_hw").and_then(|v| v.as_str()) == Some(current_hardware_id) {
            let old_hw = value
                .get("old_hw")
                .and_then(|v| v.as_str())
                .ok_or_else(|| format!("hardware_replaced event {event_id} is missing old_hw"))?;
            return Ok(ReplaceEvent {
                at: event_at,
                old_hw: old_hw.to_string(),
            });
        }
    }
    Err(format!(
        "no hardware_replaced event for {} with new_hw={current_hardware_id}",
        system_id.to_text()
    )
    .into())
}

pub fn run_replace_undo(conn: &Connection, args: ReplaceUndoArgs) -> AppResult<()> {
    let sid = ledger::SystemId::from_text(&args.system_id_text)?;

    let rows = super::devices::mutate(conn, |tx| {
        if iotkit_core_publish::store::archive_target_registered(tx)? && !args.abandon_custody {
            return Err("refused: replace-undo retroactively quarantines rows that may already be enqueued for archive; re-run with --abandon-custody to force".into());
        }
        let current = ledger::get_device(tx, &sid)?
            .filter(|row| row.state != ledger::DeviceState::Retired)
            .ok_or_else(|| format!("non-retired device {} not found", sid.to_text()))?;
        let replace_event =
            latest_replace_event_for_current_hardware(tx, &sid, &current.hardware_id)?;
        if replace_event.old_hw != args.old_hardware_id {
            return Err(format!(
                "old_hw mismatch for undo: latest hardware_replaced old_hw={} but --old-hardware-id={}",
                replace_event.old_hw, args.old_hardware_id
            )
            .into());
        }
        let to = now_ms();
        let since = args.since.unwrap_or(replace_event.at);
        if since > to {
            return Err(format!("--since must not be in the future: {since} > {to}").into());
        }
        if let Some(since) = args.since
            && since > replace_event.at
        {
            return Err(format!(
                "--since {since} is after the replace event at {}; it would leave contaminated rows unmarked",
                replace_event.at
            )
            .into());
        }
        if let Some(alive) = ledger::find_alive_by_hardware_id(tx, &args.old_hardware_id)?
            && alive.system_id != sid
        {
            return Err(ledger::LedgerError::HardwareIdInUse(args.old_hardware_id.clone()).into());
        }
        tx.execute(
            "UPDATE devices SET hardware_id = ?1 WHERE system_id = ?2 AND state != 'retired'",
            params![&replace_event.old_hw, sid.as_bytes().to_vec()],
        )?;
        let series_ids = ledger::list_series_for_device(tx, &sid)?
            .into_iter()
            .map(|row| row.series_id)
            .collect::<Vec<_>>();
        let rows = ts_query::mark_readings_quarantined(tx, &series_ids, since, to)?;
        if !series_ids.is_empty() {
            let placeholders = std::iter::repeat_n("?", series_ids.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "DELETE FROM publication_log
                 WHERE reading_seq IN (
                     SELECT seq FROM readings
                     WHERE series_id IN ({placeholders})
                       AND received_at BETWEEN ? AND ?
                       AND quarantined = 1
                 )"
            );
            tx.execute(
                &sql,
                params_from_iter(series_ids.iter().copied().chain([since, to])),
            )?;
        }
        let detail = serde_json::json!({
            "old_hw": replace_event.old_hw,
            "new_hw": current.hardware_id,
            "range": {
                "from_received_ms": since,
                "to_received_ms": to,
            },
            "rows": rows,
            "abandon_custody": args.abandon_custody,
        });
        ledger::record_event(
            tx,
            "hardware_replace_undone",
            Some(&sid),
            &detail.to_string(),
        )?;
        Ok(rows)
    })?;
    println!("{rows}");
    Ok(())
}
