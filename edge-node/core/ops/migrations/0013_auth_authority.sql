CREATE TABLE auth_state (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  auth_generation INTEGER NOT NULL DEFAULT 0 CHECK (auth_generation >= 0),
  auth_epoch TEXT NOT NULL,
  recovery_required INTEGER NOT NULL DEFAULT 0 CHECK (recovery_required IN (0, 1)),
  clock_floor_ms INTEGER NOT NULL DEFAULT 0 CHECK (clock_floor_ms >= 0),
  clock_evidence_source TEXT,
  clock_evidence_at_ms INTEGER,
  manual_evidence_seq INTEGER NOT NULL DEFAULT 0 CHECK (manual_evidence_seq >= 0)
);

INSERT INTO auth_state (id, auth_epoch)
VALUES (1, lower(hex(randomblob(16))));

ALTER TABLE operator_tokens
  ADD COLUMN auth_generation INTEGER NOT NULL DEFAULT 0;

CREATE INDEX idx_operator_tokens_authority
  ON operator_tokens(auth_generation, revoked_at, expires_at);
