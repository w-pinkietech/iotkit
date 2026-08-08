ALTER TABLE raw_records ADD COLUMN series_key TEXT;

UPDATE raw_records
SET series_key=convert_from(record_json,'UTF8')::jsonb->>'series_key'
WHERE convert_from(record_json,'UTF8')::jsonb->>'family'='measurement'
  AND jsonb_typeof(convert_from(record_json,'UTF8')::jsonb->'series_key')='string'
  AND convert_from(record_json,'UTF8')::jsonb->>'series_key'<>'';

-- The read predicate rechecks full series_key after this fixed-width discriminator.
CREATE INDEX ix_raw_records_preview_signal_received
    ON raw_records(edge_node_id,md5(series_key),received_at DESC,ledger_epoch DESC,pub_seq DESC);
