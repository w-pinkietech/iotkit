ALTER TABLE auth_state
  ADD COLUMN ownership_ever_established INTEGER NOT NULL DEFAULT 0
    CHECK (ownership_ever_established IN (0, 1));

UPDATE auth_state
SET ownership_ever_established = 1
WHERE id = 1
  AND (
    recovery_required = 1
    OR EXISTS (SELECT 1 FROM admin_credential WHERE id = 1)
  );

CREATE TABLE tls_identity (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  generation INTEGER NOT NULL CHECK (generation > 0),
  fingerprint TEXT NOT NULL CHECK (length(fingerprint) > 0),
  initialized_at INTEGER NOT NULL
);

CREATE TABLE restore_receipts (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  snapshot_sha256 TEXT NOT NULL CHECK (length(snapshot_sha256) = 64),
  old_ledger_generation INTEGER NOT NULL,
  new_ledger_generation INTEGER NOT NULL,
  old_ledger_epoch TEXT,
  new_ledger_epoch TEXT NOT NULL,
  old_auth_generation INTEGER NOT NULL,
  new_auth_generation INTEGER NOT NULL,
  old_auth_epoch TEXT NOT NULL,
  new_auth_epoch TEXT NOT NULL,
  committed_at INTEGER NOT NULL
);
