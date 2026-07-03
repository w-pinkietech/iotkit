use clap::Args;
use iotkit_core_timeseries::query as ts_query;
use rusqlite::Connection;
use std::path::PathBuf;

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
    let rows = ts_query::query_readings_v3(conn, args.series_id, args.from, args.to, u32::MAX, false)?;
    let mut out = std::fs::File::create(args.out)?;
    ts_query::export_csv(&mut out, &rows)?;
    Ok(())
}
