use clap::Args;
use iotkit_core_timeseries::query as ts_query;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Args)]
pub struct QueryArgs {
    pub series_id: i64,
    #[arg(long)]
    pub from: i64,
    #[arg(long)]
    pub to: i64,
    #[arg(long, default_value_t = 100)]
    pub limit: u32,
    #[arg(long)]
    pub quarantined: bool,
}

#[derive(Args)]
pub struct AggregateArgs {
    pub series_id: i64,
    #[arg(long)]
    pub from: i64,
    #[arg(long)]
    pub to: i64,
    #[arg(long)]
    pub bucket: i64,
}

#[derive(Args)]
pub struct ExportArgs {
    pub series_id: i64,
    #[arg(long)]
    pub from: i64,
    #[arg(long)]
    pub to: i64,
    #[arg(long)]
    pub out: PathBuf,
}

#[derive(Args)]
pub struct HealthArgs {
    #[arg(long)]
    pub path: Option<PathBuf>,
}

pub fn run_query(conn: &Connection, args: QueryArgs) -> AppResult<()> {
    let rows = ts_query::query_readings_v3(
        conn,
        args.series_id,
        args.from,
        args.to,
        args.limit,
        args.quarantined,
    )?;
    for row in rows {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            row.seq,
            row.event_time,
            row.event_time_source,
            row.quarantined as i32,
            serde_json::to_string(&row.values)?
        );
    }
    Ok(())
}

pub fn run_aggregate(conn: &Connection, args: AggregateArgs) -> AppResult<()> {
    let rows = ts_query::aggregate_readings_v3(
        conn,
        args.series_id,
        args.from,
        args.to,
        args.bucket,
        false,
    )?;
    for row in rows {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            row.bucket_start, row.count, row.min, row.max, row.avg
        );
    }
    Ok(())
}

pub fn run_export(conn: &Connection, args: ExportArgs) -> AppResult<()> {
    let rows =
        ts_query::query_readings_v3(conn, args.series_id, args.from, args.to, u32::MAX, false)?;
    let mut out = std::fs::File::create(args.out)?;
    ts_query::export_csv(&mut out, &rows)?;
    Ok(())
}

pub fn run_health(db_path: &Path, args: HealthArgs) -> AppResult<()> {
    let path = args
        .path
        .unwrap_or_else(|| default_health_json_path(db_path));
    let text = std::fs::read_to_string(&path)?;
    let json: serde_json::Value = serde_json::from_str(&text)?;
    let written_at = json["written_at"].as_i64().unwrap_or(0);
    let age_ms = now_ms().saturating_sub(written_at);
    if age_ms > 5 * 60 * 1000 {
        println!("STALE (daemon down?)");
    } else {
        println!("OK");
    }
    println!("path={}", path.display());
    println!("epoch={}", json["epoch"].as_str().unwrap_or(""));
    println!(
        "collector_alive={}",
        json["collector_alive"].as_bool().unwrap_or(false)
    );
    println!(
        "db size_bytes={} disk_available_bytes={} watermark_exceeded={}",
        json["db"]["size_bytes"].as_u64().unwrap_or(0),
        json["db"]["disk_available_bytes"].as_u64().unwrap_or(0),
        json["db"]["watermark_exceeded"].as_bool().unwrap_or(false)
    );
    println!(
        "retention days={} last_purge_at={} last_purged_rows={}",
        json["retention"]["days"].as_u64().unwrap_or(0),
        json["retention"]["last_purge_at"]
            .as_i64()
            .map(|n| n.to_string())
            .unwrap_or_else(|| "null".to_string()),
        json["retention"]["last_purged_rows"].as_u64().unwrap_or(0)
    );
    if let Some(entries) = json["publish"].as_array() {
        for entry in entries {
            let last_error = entry["last_error"].as_str().unwrap_or("-");
            println!(
                "publish target={} cursor={} backlog={} last_push_at={} last_error={}",
                entry["target_id"].as_str().unwrap_or(""),
                json_number_text(&entry["cursor_pub_seq"], "0"),
                json_number_text(&entry["backlog"], "0"),
                json_number_text(&entry["last_push_at"], "-"),
                last_error
            );
        }
    }
    Ok(())
}

fn default_health_json_path(db_path: &Path) -> PathBuf {
    match db_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join("health.json"),
        _ => PathBuf::from("health.json"),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn json_number_text(value: &serde_json::Value, default: &str) -> String {
    value
        .as_i64()
        .map(|n| n.to_string())
        .or_else(|| value.as_u64().map(|n| n.to_string()))
        .unwrap_or_else(|| default.to_string())
}
