CREATE TRIGGER descriptor_positional_model_insert
AFTER INSERT ON positional_device_models
BEGIN
    UPDATE ledger_meta
    SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
    WHERE key = 'descriptor_revision';
END;

CREATE TRIGGER descriptor_positional_model_delete
AFTER DELETE ON positional_device_models
BEGIN
    UPDATE ledger_meta
    SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
    WHERE key = 'descriptor_revision';
END;

CREATE TRIGGER descriptor_positional_model_update
AFTER UPDATE OF model_id ON positional_device_models
WHEN OLD.model_id IS NOT NEW.model_id
BEGIN
    UPDATE ledger_meta
    SET value = CAST(CAST(value AS INTEGER) + 1 AS TEXT)
    WHERE key = 'descriptor_revision';
END;
