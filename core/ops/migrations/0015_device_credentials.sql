ALTER TABLE auth_state
  ADD COLUMN device_credential_generation INTEGER NOT NULL DEFAULT 0
    CHECK (device_credential_generation >= 0);

CREATE UNIQUE INDEX idx_auth_state_epoch ON auth_state(auth_epoch);

-- These bootstrap values are deliberately minimal positive fail-closed values. They are not
-- measured product defaults: construction-tier configuration must install local network measurements.
CREATE TABLE device_flow_classes (
  flow_class TEXT PRIMARY KEY CHECK (flow_class IN ('low', 'default', 'high')),
  steady_units INTEGER NOT NULL CHECK (steady_units > 0),
  burst_units INTEGER NOT NULL CHECK (burst_units > 0)
);

INSERT INTO device_flow_classes (flow_class, steady_units, burst_units) VALUES
  ('low', 1, 1),
  ('default', 1, 1),
  ('high', 1, 1);

CREATE TABLE device_capacity (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  steady_units INTEGER NOT NULL CHECK (steady_units > 0),
  burst_units INTEGER NOT NULL CHECK (burst_units > 0),
  stale_after_ms INTEGER NOT NULL CHECK (stale_after_ms > 0)
);

INSERT INTO device_capacity (id, steady_units, burst_units, stale_after_ms)
VALUES (1, 1, 1, 1);

CREATE TABLE device_ingest_principals (
  principal_id TEXT PRIMARY KEY CHECK (length(principal_id) BETWEEN 8 AND 128),
  device_system_id BLOB NOT NULL UNIQUE REFERENCES devices(system_id),
  flow_class TEXT NOT NULL REFERENCES device_flow_classes(flow_class),
  profile TEXT NOT NULL CHECK (profile = 'simple_bearer'),
  created_at INTEGER NOT NULL
);

CREATE TABLE device_principal_scopes (
  principal_id TEXT NOT NULL REFERENCES device_ingest_principals(principal_id) ON DELETE CASCADE,
  system_id BLOB NOT NULL REFERENCES devices(system_id),
  PRIMARY KEY (principal_id, system_id)
);

CREATE TRIGGER device_scope_requires_registered_system
BEFORE INSERT ON device_principal_scopes
WHEN NOT EXISTS (
  SELECT 1 FROM devices
  WHERE system_id = NEW.system_id AND state != 'retired'
)
BEGIN
  SELECT RAISE(ABORT, 'device scope requires a registered non-retired system_id');
END;

CREATE TRIGGER device_scope_count_limit
BEFORE INSERT ON device_principal_scopes
WHEN (SELECT COUNT(*) FROM device_principal_scopes
      WHERE principal_id = NEW.principal_id) >= 64
BEGIN
  SELECT RAISE(ABORT, 'principal scope limit exceeded');
END;

CREATE TRIGGER device_scope_update_count_limit
BEFORE UPDATE OF principal_id ON device_principal_scopes
WHEN OLD.principal_id != NEW.principal_id
  AND (SELECT COUNT(*) FROM device_principal_scopes
       WHERE principal_id = NEW.principal_id) >= 64
BEGIN
  SELECT RAISE(ABORT, 'principal scope limit exceeded');
END;

CREATE TABLE device_credentials (
  credential_id TEXT PRIMARY KEY CHECK (length(credential_id) BETWEEN 8 AND 128),
  principal_id TEXT NOT NULL REFERENCES device_ingest_principals(principal_id) ON DELETE CASCADE,
  token_hash BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
  auth_epoch TEXT NOT NULL REFERENCES auth_state(auth_epoch) DEFERRABLE INITIALLY DEFERRED,
  state TEXT NOT NULL CHECK (state IN ('current', 'pending', 'revoked')),
  issued_at INTEGER NOT NULL,
  last_used_at INTEGER,
  proven_at INTEGER,
  confirmed_at INTEGER,
  revoked_at INTEGER,
  issue_reason TEXT NOT NULL CHECK (
    issue_reason IN ('device_commissioning', 'manual_issue', 'credential_reissue')
  ),
  revoke_reason TEXT CHECK (
    revoke_reason IS NULL OR revoke_reason IN (
      'credential_confirmed', 'pending_abandoned', 'operator_revoked',
      'hardware_replaced', 'device_retired'
    )
  ),
  CHECK (last_used_at IS NULL OR last_used_at >= issued_at),
  CHECK (proven_at IS NULL OR proven_at >= issued_at),
  CHECK (confirmed_at IS NULL OR (proven_at IS NOT NULL AND confirmed_at >= proven_at)),
  CHECK (revoked_at IS NULL OR (
    revoked_at >= issued_at
    AND (proven_at IS NULL OR revoked_at >= proven_at)
    AND (confirmed_at IS NULL OR revoked_at >= confirmed_at)
  )),
  CHECK (
    (state IN ('current', 'pending') AND revoked_at IS NULL AND revoke_reason IS NULL)
    OR (state = 'revoked' AND revoked_at IS NOT NULL AND revoke_reason IS NOT NULL)
  ),
  CHECK (state != 'pending' OR confirmed_at IS NULL),
  CHECK (revoke_reason != 'pending_abandoned' OR confirmed_at IS NULL),
  CHECK (issue_reason != 'credential_reissue' OR state != 'current' OR confirmed_at IS NOT NULL)
);

CREATE UNIQUE INDEX idx_device_credentials_one_current
  ON device_credentials(principal_id) WHERE state = 'current';
CREATE UNIQUE INDEX idx_device_credentials_one_pending
  ON device_credentials(principal_id) WHERE state = 'pending';
CREATE INDEX idx_device_credentials_auth
  ON device_credentials(token_hash, auth_epoch, state);
CREATE INDEX idx_device_credentials_stale
  ON device_credentials(state, last_used_at, issued_at);

-- This view is the single SQL definition of live declared authority. Rust capacity and health
-- queries consume it too: a principal must have a live device, live credential, and usable scope.
CREATE VIEW live_device_ingest_principals AS
SELECT p.*
FROM device_ingest_principals p
JOIN devices d ON d.system_id = p.device_system_id AND d.state != 'retired'
WHERE EXISTS (
  SELECT 1 FROM device_credentials c
  WHERE c.principal_id = p.principal_id AND c.state IN ('current', 'pending')
)
AND EXISTS (
  SELECT 1
  FROM device_principal_scopes s
  JOIN devices sd ON sd.system_id = s.system_id AND sd.state != 'retired'
  WHERE s.principal_id = p.principal_id
);

CREATE VIEW live_device_capacity_classes AS
SELECT f.flow_class, f.steady_units, f.burst_units,
       (SELECT COUNT(*) FROM live_device_ingest_principals p
        WHERE p.flow_class = f.flow_class) AS principal_count
FROM device_flow_classes f;

CREATE TABLE capacity_debt (
  debt_id INTEGER PRIMARY KEY AUTOINCREMENT,
  approved_at INTEGER NOT NULL,
  changed_at INTEGER NOT NULL,
  approved_by TEXT NOT NULL CHECK (length(approved_by) BETWEEN 1 AND 128),
  operation TEXT NOT NULL CHECK (operation IN (
    'device_add', 'credential_issue', 'flow_class_change', 'authority_configure'
  )),
  required_steady_units INTEGER NOT NULL CHECK (required_steady_units >= 0),
  required_burst_units INTEGER NOT NULL CHECK (required_burst_units >= 0),
  capacity_steady_units INTEGER NOT NULL CHECK (capacity_steady_units > 0),
  capacity_burst_units INTEGER NOT NULL CHECK (capacity_burst_units > 0),
  recovered_at INTEGER CHECK (recovered_at IS NULL OR recovered_at >= changed_at)
);

CREATE UNIQUE INDEX idx_capacity_debt_one_active
  ON capacity_debt((1)) WHERE recovered_at IS NULL;

CREATE TRIGGER device_principal_insert_generation
AFTER INSERT ON device_ingest_principals
BEGIN
  UPDATE auth_state SET device_credential_generation = device_credential_generation + 1 WHERE id = 1;
END;

CREATE TRIGGER device_principal_material_generation
AFTER UPDATE OF principal_id, device_system_id, flow_class, profile ON device_ingest_principals
WHEN OLD.principal_id != NEW.principal_id
  OR OLD.device_system_id != NEW.device_system_id
  OR OLD.flow_class != NEW.flow_class
  OR OLD.profile != NEW.profile
BEGIN
  UPDATE auth_state SET device_credential_generation = device_credential_generation + 1 WHERE id = 1;
END;

CREATE TRIGGER device_principal_delete_generation
AFTER DELETE ON device_ingest_principals
BEGIN
  UPDATE auth_state SET device_credential_generation = device_credential_generation + 1 WHERE id = 1;
END;

CREATE TRIGGER device_scope_insert_generation
AFTER INSERT ON device_principal_scopes
BEGIN
  UPDATE auth_state SET device_credential_generation = device_credential_generation + 1 WHERE id = 1;
END;

CREATE TRIGGER device_scope_delete_generation
AFTER DELETE ON device_principal_scopes
BEGIN
  UPDATE auth_state SET device_credential_generation = device_credential_generation + 1 WHERE id = 1;
END;

CREATE TRIGGER device_scope_update_generation
AFTER UPDATE OF principal_id, system_id ON device_principal_scopes
WHEN OLD.principal_id != NEW.principal_id OR OLD.system_id != NEW.system_id
BEGIN
  UPDATE auth_state SET device_credential_generation = device_credential_generation + 1 WHERE id = 1;
END;

CREATE TRIGGER device_credential_insert_generation
AFTER INSERT ON device_credentials
BEGIN
  UPDATE auth_state SET device_credential_generation = device_credential_generation + 1 WHERE id = 1;
END;

CREATE TRIGGER device_credential_state_generation
AFTER UPDATE OF credential_id, principal_id, token_hash, auth_epoch, state, proven_at, confirmed_at ON device_credentials
WHEN OLD.state != NEW.state
  OR OLD.credential_id != NEW.credential_id
  OR OLD.principal_id != NEW.principal_id
  OR OLD.token_hash != NEW.token_hash
  OR OLD.auth_epoch != NEW.auth_epoch
  OR OLD.proven_at IS NOT NEW.proven_at
  OR OLD.confirmed_at IS NOT NEW.confirmed_at
BEGIN
  UPDATE auth_state SET device_credential_generation = device_credential_generation + 1 WHERE id = 1;
END;

CREATE TRIGGER device_credential_delete_generation
AFTER DELETE ON device_credentials
BEGIN
  UPDATE auth_state SET device_credential_generation = device_credential_generation + 1 WHERE id = 1;
END;

CREATE TRIGGER device_flow_class_weight_generation
AFTER UPDATE OF steady_units, burst_units ON device_flow_classes
WHEN OLD.steady_units != NEW.steady_units OR OLD.burst_units != NEW.burst_units
BEGIN
  UPDATE auth_state SET device_credential_generation = device_credential_generation + 1 WHERE id = 1;
END;

CREATE TRIGGER device_capacity_generation
AFTER UPDATE OF steady_units, burst_units ON device_capacity
WHEN OLD.steady_units != NEW.steady_units OR OLD.burst_units != NEW.burst_units
BEGIN
  UPDATE auth_state SET device_credential_generation = device_credential_generation + 1 WHERE id = 1;
END;

CREATE TRIGGER device_credential_live_removal_reconciles_capacity_debt
AFTER UPDATE OF state ON device_credentials
WHEN OLD.state IN ('current','pending') AND NEW.state = 'revoked'
  AND NOT EXISTS (
    SELECT 1 FROM device_credentials c
    WHERE c.principal_id=OLD.principal_id AND c.state IN ('current','pending')
  )
BEGIN
  SELECT CASE WHEN principal_count > 9223372036854775807 / steady_units
                    OR principal_count > 9223372036854775807 / burst_units
              THEN RAISE(ABORT, 'capacity_math_overflow') END
  FROM live_device_capacity_classes;
  WITH terms AS (
    SELECT flow_class, principal_count * steady_units AS steady,
           principal_count * burst_units AS burst
    FROM live_device_capacity_classes
  ), totals AS (
    SELECT COALESCE(MAX(CASE WHEN flow_class='low' THEN steady END),0) AS low_s,
           COALESCE(MAX(CASE WHEN flow_class='default' THEN steady END),0) AS default_s,
           COALESCE(MAX(CASE WHEN flow_class='high' THEN steady END),0) AS high_s,
           COALESCE(MAX(CASE WHEN flow_class='low' THEN burst END),0) AS low_b,
           COALESCE(MAX(CASE WHEN flow_class='default' THEN burst END),0) AS default_b,
           COALESCE(MAX(CASE WHEN flow_class='high' THEN burst END),0) AS high_b
    FROM terms
  )
  SELECT CASE
           WHEN low_s > 9223372036854775807 - default_s
             THEN RAISE(ABORT, 'capacity_math_overflow')
           WHEN low_s + default_s > 9223372036854775807 - high_s
             THEN RAISE(ABORT, 'capacity_math_overflow')
           WHEN low_b > 9223372036854775807 - default_b
             THEN RAISE(ABORT, 'capacity_math_overflow')
           WHEN low_b + default_b > 9223372036854775807 - high_b
             THEN RAISE(ABORT, 'capacity_math_overflow')
         END
  FROM totals;
  UPDATE capacity_debt
  SET changed_at = MAX(changed_at, NEW.revoked_at),
      required_steady_units = (
        SELECT COALESCE(SUM(f.steady_units), 0)
        FROM live_device_ingest_principals p
        JOIN device_flow_classes f ON f.flow_class=p.flow_class
      ),
      required_burst_units = (
        SELECT COALESCE(SUM(f.burst_units), 0)
        FROM live_device_ingest_principals p
        JOIN device_flow_classes f ON f.flow_class=p.flow_class
      ),
      capacity_steady_units = (SELECT steady_units FROM device_capacity WHERE id=1),
      capacity_burst_units = (SELECT burst_units FROM device_capacity WHERE id=1),
      recovered_at = CASE
        WHEN (
          SELECT COALESCE(SUM(f.steady_units), 0)
          FROM live_device_ingest_principals p
          JOIN device_flow_classes f ON f.flow_class=p.flow_class
        ) <= (SELECT steady_units FROM device_capacity WHERE id=1)
        AND (
          SELECT COALESCE(SUM(f.burst_units), 0)
          FROM live_device_ingest_principals p
          JOIN device_flow_classes f ON f.flow_class=p.flow_class
        ) <= (SELECT burst_units FROM device_capacity WHERE id=1)
        THEN MAX(changed_at, NEW.revoked_at)
        ELSE NULL
      END
  WHERE recovered_at IS NULL;
  INSERT INTO ledger_events (at, kind, system_id, detail)
  SELECT
    MAX(CAST(unixepoch('subsec') * 1000 AS INTEGER), NEW.revoked_at),
    'capacity_debt',
    NULL,
    '{"code":"capacity_debt_reconciled_after_authority_removal"}'
  WHERE changes() > 0;
END;

CREATE TRIGGER device_scope_delete_reconciles_capacity_debt
AFTER DELETE ON device_principal_scopes
BEGIN
  SELECT CASE WHEN principal_count > 9223372036854775807 / steady_units
                    OR principal_count > 9223372036854775807 / burst_units
              THEN RAISE(ABORT, 'capacity_math_overflow') END
  FROM live_device_capacity_classes;
  WITH terms AS (
    SELECT flow_class, principal_count * steady_units AS steady,
           principal_count * burst_units AS burst
    FROM live_device_capacity_classes
  ), totals AS (
    SELECT COALESCE(MAX(CASE WHEN flow_class='low' THEN steady END),0) AS low_s,
           COALESCE(MAX(CASE WHEN flow_class='default' THEN steady END),0) AS default_s,
           COALESCE(MAX(CASE WHEN flow_class='high' THEN steady END),0) AS high_s,
           COALESCE(MAX(CASE WHEN flow_class='low' THEN burst END),0) AS low_b,
           COALESCE(MAX(CASE WHEN flow_class='default' THEN burst END),0) AS default_b,
           COALESCE(MAX(CASE WHEN flow_class='high' THEN burst END),0) AS high_b
    FROM terms
  )
  SELECT CASE
           WHEN low_s > 9223372036854775807 - default_s
             THEN RAISE(ABORT, 'capacity_math_overflow')
           WHEN low_s + default_s > 9223372036854775807 - high_s
             THEN RAISE(ABORT, 'capacity_math_overflow')
           WHEN low_b > 9223372036854775807 - default_b
             THEN RAISE(ABORT, 'capacity_math_overflow')
           WHEN low_b + default_b > 9223372036854775807 - high_b
             THEN RAISE(ABORT, 'capacity_math_overflow')
         END
  FROM totals;
  UPDATE capacity_debt
  SET changed_at = MAX(changed_at, CAST(unixepoch('subsec') * 1000 AS INTEGER)),
      required_steady_units = (
        SELECT COALESCE(SUM(f.steady_units), 0)
        FROM live_device_ingest_principals p
        JOIN device_flow_classes f ON f.flow_class=p.flow_class
      ),
      required_burst_units = (
        SELECT COALESCE(SUM(f.burst_units), 0)
        FROM live_device_ingest_principals p
        JOIN device_flow_classes f ON f.flow_class=p.flow_class
      ),
      capacity_steady_units = (SELECT steady_units FROM device_capacity WHERE id=1),
      capacity_burst_units = (SELECT burst_units FROM device_capacity WHERE id=1),
      recovered_at = CASE
        WHEN (
          SELECT COALESCE(SUM(f.steady_units), 0)
          FROM live_device_ingest_principals p
          JOIN device_flow_classes f ON f.flow_class=p.flow_class
        ) <= (SELECT steady_units FROM device_capacity WHERE id=1)
        AND (
          SELECT COALESCE(SUM(f.burst_units), 0)
          FROM live_device_ingest_principals p
          JOIN device_flow_classes f ON f.flow_class=p.flow_class
        ) <= (SELECT burst_units FROM device_capacity WHERE id=1)
        THEN MAX(changed_at, CAST(unixepoch('subsec') * 1000 AS INTEGER))
        ELSE NULL
      END
  WHERE recovered_at IS NULL;
  INSERT INTO ledger_events (at, kind, system_id, detail)
  SELECT CAST(unixepoch('subsec') * 1000 AS INTEGER),
         'capacity_debt', NULL,
         '{"code":"capacity_debt_reconciled_after_scope_removal"}'
  WHERE changes() > 0;
END;

CREATE TRIGGER device_hardware_change_revokes_credentials
AFTER UPDATE OF hardware_id ON devices
WHEN OLD.hardware_id != NEW.hardware_id
BEGIN
  UPDATE auth_state SET device_credential_generation = device_credential_generation + 1 WHERE id = 1;
  UPDATE device_credentials
  SET state = 'revoked',
      revoked_at = MAX(
        CAST(unixepoch('subsec') * 1000 AS INTEGER),
        issued_at,
        COALESCE(last_used_at, issued_at),
        COALESCE(proven_at, issued_at),
        COALESCE(confirmed_at, issued_at)
      ),
      revoke_reason = 'hardware_replaced'
  WHERE principal_id IN (
    SELECT principal_id FROM device_ingest_principals WHERE device_system_id = NEW.system_id
  ) AND state != 'revoked';
  INSERT INTO ledger_events (at, kind, system_id, detail)
  VALUES (
    CAST(unixepoch('subsec') * 1000 AS INTEGER),
    'device_credential_authority', NEW.system_id,
    '{"code":"credentials_revoked","reason_code":"hardware_replaced"}'
  );
END;

CREATE TRIGGER device_retire_closes_authority
AFTER UPDATE OF state ON devices
WHEN OLD.state != 'retired' AND NEW.state = 'retired'
BEGIN
  UPDATE auth_state SET device_credential_generation = device_credential_generation + 1 WHERE id = 1;
  UPDATE device_credentials
  SET state = 'revoked',
      revoked_at = MAX(
        CAST(unixepoch('subsec') * 1000 AS INTEGER),
        issued_at,
        COALESCE(last_used_at, issued_at),
        COALESCE(proven_at, issued_at),
        COALESCE(confirmed_at, issued_at)
      ),
      revoke_reason = 'device_retired'
  WHERE principal_id IN (
    SELECT principal_id FROM device_ingest_principals WHERE device_system_id = NEW.system_id
  ) AND state != 'revoked';
  DELETE FROM device_principal_scopes WHERE system_id = NEW.system_id;
  DELETE FROM device_principal_scopes
    WHERE principal_id IN (
      SELECT principal_id FROM device_ingest_principals WHERE device_system_id = NEW.system_id
    );
  SELECT CASE WHEN principal_count > 9223372036854775807 / steady_units
                    OR principal_count > 9223372036854775807 / burst_units
              THEN RAISE(ABORT, 'capacity_math_overflow') END
  FROM live_device_capacity_classes;
  WITH terms AS (
    SELECT flow_class, principal_count * steady_units AS steady,
           principal_count * burst_units AS burst
    FROM live_device_capacity_classes
  ), totals AS (
    SELECT COALESCE(MAX(CASE WHEN flow_class='low' THEN steady END),0) AS low_s,
           COALESCE(MAX(CASE WHEN flow_class='default' THEN steady END),0) AS default_s,
           COALESCE(MAX(CASE WHEN flow_class='high' THEN steady END),0) AS high_s,
           COALESCE(MAX(CASE WHEN flow_class='low' THEN burst END),0) AS low_b,
           COALESCE(MAX(CASE WHEN flow_class='default' THEN burst END),0) AS default_b,
           COALESCE(MAX(CASE WHEN flow_class='high' THEN burst END),0) AS high_b
    FROM terms
  )
  SELECT CASE
           WHEN low_s > 9223372036854775807 - default_s
             THEN RAISE(ABORT, 'capacity_math_overflow')
           WHEN low_s + default_s > 9223372036854775807 - high_s
             THEN RAISE(ABORT, 'capacity_math_overflow')
           WHEN low_b > 9223372036854775807 - default_b
             THEN RAISE(ABORT, 'capacity_math_overflow')
           WHEN low_b + default_b > 9223372036854775807 - high_b
             THEN RAISE(ABORT, 'capacity_math_overflow')
         END
  FROM totals;
  UPDATE capacity_debt
  SET recovered_at = CAST(unixepoch('subsec') * 1000 AS INTEGER),
      changed_at = CAST(unixepoch('subsec') * 1000 AS INTEGER)
  WHERE recovered_at IS NULL
    AND (SELECT COALESCE(SUM(f.steady_units), 0)
         FROM live_device_ingest_principals p
         JOIN device_flow_classes f ON f.flow_class = p.flow_class)
        <= (SELECT steady_units FROM device_capacity WHERE id = 1)
    AND (SELECT COALESCE(SUM(f.burst_units), 0)
         FROM live_device_ingest_principals p
         JOIN device_flow_classes f ON f.flow_class = p.flow_class)
        <= (SELECT burst_units FROM device_capacity WHERE id = 1);
  INSERT INTO ledger_events (at, kind, system_id, detail)
  SELECT
    CAST(unixepoch('subsec') * 1000 AS INTEGER),
    'capacity_debt', NEW.system_id,
    json_object(
      'code', 'capacity_debt_recovered',
      'required_steady_units',
        (SELECT COALESCE(SUM(f.steady_units), 0)
         FROM live_device_ingest_principals p
         JOIN device_flow_classes f ON f.flow_class = p.flow_class),
      'required_burst_units',
        (SELECT COALESCE(SUM(f.burst_units), 0)
         FROM live_device_ingest_principals p
         JOIN device_flow_classes f ON f.flow_class = p.flow_class)
    )
  WHERE changes() > 0;
  INSERT INTO ledger_events (at, kind, system_id, detail)
  VALUES (
    CAST(unixepoch('subsec') * 1000 AS INTEGER),
    'device_credential_authority', NEW.system_id,
    '{"code":"credentials_revoked_and_scopes_closed","reason_code":"device_retired"}'
  );
END;
