ALTER TABLE devices ADD COLUMN presentation_identifier TEXT;

INSERT INTO ledger_meta(key, value) VALUES ('descriptor_revision', '1');

CREATE TRIGGER descriptor_devices_insert
AFTER INSERT ON devices
BEGIN
    UPDATE ledger_meta
    SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
    WHERE key = 'descriptor_revision';
END;

CREATE TRIGGER descriptor_devices_delete
AFTER DELETE ON devices
BEGIN
    UPDATE ledger_meta
    SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
    WHERE key = 'descriptor_revision';
END;

CREATE TRIGGER descriptor_devices_update
AFTER UPDATE OF state, presentation_identifier ON devices
WHEN OLD.state IS NOT NEW.state
  OR OLD.presentation_identifier IS NOT NEW.presentation_identifier
BEGIN
    UPDATE ledger_meta
    SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
    WHERE key = 'descriptor_revision';
END;

CREATE TRIGGER descriptor_series_insert
AFTER INSERT ON series
BEGIN
    UPDATE ledger_meta
    SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
    WHERE key = 'descriptor_revision';
END;

CREATE TRIGGER descriptor_series_delete
AFTER DELETE ON series
BEGIN
    UPDATE ledger_meta
    SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
    WHERE key = 'descriptor_revision';
END;

CREATE TRIGGER descriptor_series_update
AFTER UPDATE OF system_id, measurement_key, channel_index, variant, quarantined ON series
WHEN OLD.system_id IS NOT NEW.system_id
  OR OLD.measurement_key IS NOT NEW.measurement_key
  OR OLD.channel_index IS NOT NEW.channel_index
  OR OLD.variant IS NOT NEW.variant
  OR OLD.quarantined IS NOT NEW.quarantined
BEGIN
    UPDATE ledger_meta
    SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
    WHERE key = 'descriptor_revision';
END;
