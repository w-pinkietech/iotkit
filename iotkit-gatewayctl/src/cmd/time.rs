use std::io;

use clap::Subcommand;
use rusqlite::Connection;

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Subcommand)]
pub enum TimeCommand {
    Confirm,
}

pub fn run_time_confirm(conn: &Connection) -> AppResult<()> {
    let clock = iotkit_core_ops::SystemClock::default();
    let current = iotkit_core_ops::Clock::wall_time_ms(&clock);
    let floor = iotkit_core_ops::ClockTrust::persisted_floor(conn)?;
    eprintln!("current_time_ms={current}");
    let current_utc =
        time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(current) * 1_000_000)
            .map_err(|_| "current local time is outside the displayable timestamp range")?;
    eprintln!("current_time_utc={current_utc}");
    eprintln!("persisted_auth_time_floor_ms={floor}");
    let confirmation_window_seconds =
        iotkit_core_ops::clock::MAX_MANUAL_CONFIRMATION_DRIFT_MS / 1_000;
    eprintln!(
        "Type 'confirm' within {confirmation_window_seconds} seconds to trust this local time for the current gateway process:"
    );
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    if input.trim() != "confirm" {
        return Err("time confirmation aborted".into());
    }
    iotkit_core_ops::confirm_time_with_clock(conn, &clock, current)?;
    println!("time trust evidence recorded");
    Ok(())
}
