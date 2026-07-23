CREATE TABLE positional_device_models (
    system_id BLOB PRIMARY KEY REFERENCES devices(system_id) ON DELETE CASCADE,
    model_id  TEXT NOT NULL CHECK (length(model_id) > 0)
);

-- Before device lists became configurable, rpi-local had exactly these two
-- fixed inventory labels. Fence those existing rows during migration so a
-- simultaneous config edit cannot silently reinterpret their history.
INSERT INTO positional_device_models(system_id, model_id)
SELECT system_id,
       CASE
           WHEN user_label = 'MCP9600 thermocouple'
                AND hardware_id LIKE '%:i2c:0x60' THEN 'mcp9600'
           WHEN user_label = 'OPT3001 illuminance'
                AND hardware_id LIKE '%:i2c:0x44' THEN 'opt3001'
       END
FROM devices
WHERE kind = 'positional'
  AND state != 'retired'
  AND (
      (user_label = 'MCP9600 thermocouple' AND hardware_id LIKE '%:i2c:0x60')
      OR
      (user_label = 'OPT3001 illuminance' AND hardware_id LIKE '%:i2c:0x44')
  );
