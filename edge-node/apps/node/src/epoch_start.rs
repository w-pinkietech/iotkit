use rusqlite::OptionalExtension;

pub fn maybe_enqueue_epoch_start(conn: &rusqlite::Connection) -> Result<(), String> {
    let current_epoch = iotkit_core_ledger::ledger_epoch(conn).map_err(|e| e.to_string())?;
    if !iotkit_core_publish::activation::publication_admitted(conn)
        .map_err(|error| error.to_string())?
    {
        tracing::debug!(
            epoch = %current_epoch,
            "skipping epoch_start annotation until Edge Node activation"
        );
        return Ok(());
    }
    let detail: Option<String> = conn
        .query_row(
            "SELECT detail
             FROM ledger_events
             WHERE kind = 'epoch_renewed'
             ORDER BY event_id DESC
             LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some(detail) = detail else {
        // First boot/pristine database: no renewal means nothing to announce.
        tracing::debug!(
            epoch = %current_epoch,
            "skipping epoch_start annotation: no epoch_renewed event"
        );
        return Ok(());
    };

    let detail: serde_json::Value = serde_json::from_str(&detail).map_err(|e| e.to_string())?;
    let Some(old_epoch) = detail.get("old_epoch").and_then(|value| value.as_str()) else {
        // Fresh-box renew: no prior epoch exists to reference.
        tracing::debug!(
            epoch = %current_epoch,
            "skipping epoch_start annotation: no string old_epoch"
        );
        return Ok(());
    };

    let payload = serde_json::json!({ "prior_epoch": old_epoch }).to_string();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0);
    let enqueued = iotkit_core_publish::store::enqueue_annotation(
        conn,
        &current_epoch,
        "epoch_start",
        &payload,
        now_ms,
    )
    .map_err(|e| e.to_string())?;
    match enqueued {
        Some(pub_seq) => tracing::info!(
            epoch = %current_epoch,
            prior_epoch = %old_epoch,
            pub_seq = pub_seq,
            "epoch_start annotation enqueued"
        ),
        None => tracing::debug!(
            epoch = %current_epoch,
            prior_epoch = %old_epoch,
            "epoch_start annotation already enqueued"
        ),
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/epoch_start_tests.rs"]
mod tests;
