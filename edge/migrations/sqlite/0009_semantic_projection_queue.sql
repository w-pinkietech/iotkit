CREATE TABLE semantic_projection_queue (
    rule_id TEXT NOT NULL,
    signal_ref TEXT NOT NULL,
    edge_node_id TEXT NOT NULL,
    ledger_epoch TEXT NOT NULL,
    pub_seq INTEGER NOT NULL CHECK(pub_seq > 0),
    received_at INTEGER NOT NULL CHECK(received_at >= 0),
    rule_created_at INTEGER NOT NULL CHECK(rule_created_at >= 0),
    revision INTEGER NOT NULL CHECK(revision > 0),
    calibration_revision INTEGER NOT NULL CHECK(calibration_revision > 0),
    PRIMARY KEY(rule_id, ledger_epoch, pub_seq),
    FOREIGN KEY(edge_node_id,ledger_epoch,pub_seq)
        REFERENCES raw_records(edge_node_id,ledger_epoch,pub_seq),
    FOREIGN KEY(rule_id,revision)
        REFERENCES semantic_rule_revisions(rule_id,revision),
    FOREIGN KEY(signal_ref,calibration_revision)
        REFERENCES semantic_calibration_revisions(signal_ref,revision)
);

INSERT INTO semantic_projection_queue(
    rule_id,signal_ref,edge_node_id,ledger_epoch,pub_seq,received_at,rule_created_at,
    revision,calibration_revision
)
SELECT rule.rule_id,rule.signal_ref,raw.edge_node_id,raw.ledger_epoch,raw.pub_seq,
       raw.received_at,rule.created_at,revision.revision,calibration.revision
FROM semantic_rules AS rule
JOIN semantic_signals AS signal ON signal.signal_ref=rule.signal_ref
JOIN raw_records AS raw ON raw.edge_node_id=signal.edge_node_id
  AND json_extract(raw.record_json,'$.series_key')=signal.series_key
JOIN semantic_rule_revisions AS revision ON revision.rule_id=rule.rule_id
  AND revision.revision=CASE
    WHEN EXISTS(SELECT 1 FROM semantic_rule_starts AS all_start
      WHERE all_start.rule_id=rule.rule_id AND all_start.ledger_epoch=raw.ledger_epoch)
    THEN COALESCE((SELECT MAX(start.revision) FROM semantic_rule_starts AS start
      WHERE start.rule_id=rule.rule_id AND start.ledger_epoch=raw.ledger_epoch
        AND raw.pub_seq>start.start_after_pub_seq),
      (SELECT MIN(start.revision)-1 FROM semantic_rule_starts AS start
        WHERE start.rule_id=rule.rule_id AND start.ledger_epoch=raw.ledger_epoch))
    ELSE rule.revision
  END
JOIN semantic_calibration_revisions AS calibration ON calibration.signal_ref=signal.signal_ref
  AND calibration.revision=CASE
    WHEN EXISTS(SELECT 1 FROM semantic_calibration_starts AS all_cal
      WHERE all_cal.signal_ref=signal.signal_ref AND all_cal.ledger_epoch=raw.ledger_epoch)
    THEN COALESCE((SELECT MAX(start.revision) FROM semantic_calibration_starts AS start
      WHERE start.signal_ref=signal.signal_ref AND start.ledger_epoch=raw.ledger_epoch
        AND raw.pub_seq>start.start_after_pub_seq),
      (SELECT MIN(start.revision)-1 FROM semantic_calibration_starts AS start
        WHERE start.signal_ref=signal.signal_ref AND start.ledger_epoch=raw.ledger_epoch))
    ELSE signal.calibration_revision
  END
WHERE json_extract(raw.record_json,'$.family')='measurement'
  AND NOT EXISTS(SELECT 1 FROM semantic_projection_receipts AS receipt
    WHERE receipt.rule_id=rule.rule_id AND receipt.ledger_epoch=raw.ledger_epoch
      AND receipt.pub_seq=raw.pub_seq)
  AND (NOT EXISTS(SELECT 1 FROM semantic_rule_ends AS finish
    WHERE finish.rule_id=rule.rule_id AND finish.ledger_epoch=raw.ledger_epoch)
    OR raw.pub_seq<=(SELECT finish.end_at_pub_seq FROM semantic_rule_ends AS finish
      WHERE finish.rule_id=rule.rule_id AND finish.ledger_epoch=raw.ledger_epoch));

CREATE INDEX ix_semantic_projection_queue_next
    ON semantic_projection_queue(
        received_at,edge_node_id,ledger_epoch,pub_seq,rule_created_at,rule_id
    );
CREATE INDEX ix_semantic_projection_queue_rule_next
    ON semantic_projection_queue(rule_id,received_at,edge_node_id,ledger_epoch,pub_seq);
CREATE INDEX ix_semantic_projection_queue_reset_boundary
    ON semantic_projection_queue(rule_id,ledger_epoch,pub_seq);
CREATE INDEX ix_semantic_counter_resets_pending_rule
    ON semantic_counter_resets(rule_id) WHERE applied_at IS NULL;
