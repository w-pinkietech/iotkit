ALTER TABLE raw_records ADD COLUMN series_key TEXT;

UPDATE raw_records
SET series_key=json_extract(record_json,'$.series_key')
WHERE json_extract(record_json,'$.family')='measurement'
  AND json_type(record_json,'$.series_key')='text'
  AND json_extract(record_json,'$.series_key')<>'';

CREATE INDEX ix_raw_records_preview_signal_received
    ON raw_records(edge_node_id,series_key,received_at DESC,ledger_epoch DESC,pub_seq DESC);
