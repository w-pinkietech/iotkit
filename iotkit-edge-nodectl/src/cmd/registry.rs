use clap::Args;
use iotkit_core_ledger as ledger;
use iotkit_core_registry as registry;
use rusqlite::{Connection, params};

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Args)]
pub struct RegistryListArgs {
    #[arg(long)]
    pub aliases: bool,
}

#[derive(Args)]
pub struct RegistryEnableArgs {
    pub measurement_key: String,
}

#[derive(Args)]
pub struct RegistryAliasArgs {
    pub alias: String,
    pub canonical_key: String,
    #[arg(long)]
    pub release_abandon_past: bool,
}

#[derive(Args)]
pub struct SeriesListArgs {
    pub system_id_text: String,
}

fn channel_mode_label(mode: registry::ChannelMode) -> &'static str {
    match mode {
        registry::ChannelMode::Single => "single",
        registry::ChannelMode::Generic => "generic",
        registry::ChannelMode::Fixed => "fixed",
    }
}

pub fn run_registry_list(conn: &Connection, args: RegistryListArgs) -> AppResult<()> {
    if args.aliases {
        for row in registry::list_aliases(conn)? {
            println!("{}\t{}\t{}", row.alias, row.measurement_key, row.alias_kind);
        }
    } else {
        for row in registry::list_entries(conn)? {
            println!(
                "{}\t{}\t{}\t{}",
                row.measurement_key,
                row.origin,
                channel_mode_label(row.channel_mode),
                row.unit_display.unwrap_or_default()
            );
        }
    }
    Ok(())
}

pub fn run_registry_enable(conn: &Connection, args: RegistryEnableArgs) -> AppResult<()> {
    let catalog = registry::standard_catalog();
    let entry = catalog
        .find(&args.measurement_key)
        .ok_or_else(|| format!("catalog entry not found: {}", args.measurement_key))?;
    let row = super::devices::mutate(conn, |tx| {
        Ok(registry::enable_entry(
            tx,
            entry,
            &catalog.catalog_version,
            "iotkit-edge-nodectl",
        )?)
    })?;
    println!("{}", row.measurement_key);
    Ok(())
}

pub fn run_registry_alias(conn: &Connection, args: RegistryAliasArgs) -> AppResult<()> {
    super::devices::mutate(conn, |tx| {
        let has_quarantined: bool = tx.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM series
                 WHERE measurement_key = ?1 AND quarantined = 1
             )",
            params![&args.alias],
            |row| row.get(0),
        )?;
        let has_archive =
            has_quarantined && iotkit_core_publish::store::archive_target_registered(tx)?;
        if has_archive && !args.release_abandon_past {
            return Err("refused: releasing past quarantine while an archive target is registered would abandon custody of already-archived data; re-run with --release-abandon-past to force".into());
        }
        registry::define_alias(
            tx,
            &args.alias,
            &args.canonical_key,
            registry::AliasKind::LocationMapping,
        )?;
        if has_archive && args.release_abandon_past {
            ledger::record_event(
                tx,
                "quarantine_release_abandon_past",
                None,
                &serde_json::json!({
                    "alias": &args.alias,
                    "canonical": &args.canonical_key,
                    "abandon_past": true,
                })
                .to_string(),
            )?;
        }
        Ok(())
    })?;
    println!("{}\t{}", args.alias, args.canonical_key);
    Ok(())
}

pub fn run_series_list(conn: &Connection, args: SeriesListArgs) -> AppResult<()> {
    let sid = ledger::SystemId::from_text(&args.system_id_text)?;
    for row in ledger::list_series_for_device(conn, &sid)? {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            row.series_id,
            row.measurement_key,
            row.channel_index,
            row.variant,
            row.quarantined,
            row.quarantine_reason.unwrap_or_default()
        );
    }
    Ok(())
}
