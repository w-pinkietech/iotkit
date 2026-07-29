CREATE TABLE edge_node_recovery_candidate (
  singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
  state TEXT NOT NULL CHECK (state = 'durably_fenced_candidate'),
  recovery_id TEXT NOT NULL,
  candidate_instance_id TEXT NOT NULL UNIQUE,
  backup_id TEXT,
  edge_id TEXT NOT NULL,
  edge_node_id TEXT NOT NULL,
  old_ledger_epoch TEXT NOT NULL,
  proposed_new_epoch TEXT NOT NULL,
  credential_generation INTEGER NOT NULL CHECK (credential_generation >= 0),
  handoff_schema_version INTEGER NOT NULL CHECK (handoff_schema_version = 1),
  installed_at_ms INTEGER NOT NULL
);

CREATE TRIGGER edge_node_recovery_candidate_immutable
BEFORE UPDATE ON edge_node_recovery_candidate
BEGIN
  SELECT RAISE(ABORT, 'recovery candidate is immutable');
END;

CREATE TABLE edge_node_backup_attempts (
  attempt_id TEXT PRIMARY KEY,
  backup_id TEXT NOT NULL UNIQUE,
  state TEXT NOT NULL CHECK (state IN ('started', 'success', 'failed')),
  reason_code TEXT,
  artifact_name TEXT NOT NULL UNIQUE,
  artifact_length INTEGER,
  edge_node_id TEXT NOT NULL,
  ledger_epoch TEXT,
  accepted_cursor INTEGER,
  allocation_high_water INTEGER,
  started_at_ms INTEGER NOT NULL,
  artifact_created_at_ms INTEGER,
  completed_at_ms INTEGER,
  CHECK (
    (state = 'started' AND reason_code IS NULL AND completed_at_ms IS NULL)
    OR
    (state = 'success' AND reason_code = 'ok'
      AND artifact_length IS NOT NULL AND ledger_epoch IS NOT NULL
      AND accepted_cursor IS NOT NULL AND allocation_high_water IS NOT NULL
      AND artifact_created_at_ms IS NOT NULL AND completed_at_ms IS NOT NULL)
    OR
    (state = 'failed' AND reason_code IS NOT NULL
      AND reason_code <> 'ok' AND completed_at_ms IS NOT NULL)
  )
);

CREATE TRIGGER edge_node_backup_attempts_forward_only
BEFORE UPDATE ON edge_node_backup_attempts
WHEN NOT (
  OLD.state = 'started'
  AND NEW.state IN ('success', 'failed')
)
BEGIN
  SELECT RAISE(ABORT, 'backup attempt transition is not allowed');
END;
