CREATE TRIGGER descriptor_registry_insert
AFTER INSERT ON registry_entries
BEGIN
    UPDATE ledger_meta
    SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
    WHERE key = 'descriptor_revision';
END;

CREATE TRIGGER descriptor_registry_delete
AFTER DELETE ON registry_entries
BEGIN
    UPDATE ledger_meta
    SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
    WHERE key = 'descriptor_revision';
END;

CREATE TRIGGER descriptor_registry_update
AFTER UPDATE OF measurement_key, unit_ucum, value_type ON registry_entries
WHEN OLD.measurement_key IS NOT NEW.measurement_key
  OR OLD.unit_ucum IS NOT NEW.unit_ucum
  OR OLD.value_type IS NOT NEW.value_type
BEGIN
    UPDATE ledger_meta
    SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
    WHERE key = 'descriptor_revision';
END;

