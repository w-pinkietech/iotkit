-- Task 4: one durable owner for desired/applied ingress listener state. The compiled readiness
-- gate is deliberately absent from this schema: Task 6 must change code, not configuration.
CREATE TABLE ingress_listener_config (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  desired_generation INTEGER NOT NULL DEFAULT 0 CHECK (desired_generation >= 0),
  applied_generation INTEGER NOT NULL DEFAULT 0 CHECK (
    applied_generation >= 0 AND applied_generation <= desired_generation
  ),
  enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
  bind_addr TEXT NOT NULL DEFAULT '127.0.0.1:0',
  interface TEXT NOT NULL DEFAULT 'disabled' CHECK (length(interface) BETWEEN 1 AND 64),
  local_ingress_cidrs TEXT NOT NULL DEFAULT '[]',
  mode TEXT NOT NULL DEFAULT 'tls' CHECK (mode IN ('tls', 'private_plaintext')),
  desired_tls_generation INTEGER CHECK (desired_tls_generation IS NULL OR desired_tls_generation > 0),
  desired_tls_fingerprint TEXT,
  applied_bind_addr TEXT,
  applied_interface TEXT,
  applied_local_ingress_cidrs TEXT,
  applied_mode TEXT CHECK (applied_mode IS NULL OR applied_mode IN ('tls', 'private_plaintext')),
  applied_tls_generation INTEGER CHECK (applied_tls_generation IS NULL OR applied_tls_generation > 0),
  applied_tls_fingerprint TEXT,
  last_error TEXT,
  last_action TEXT NOT NULL DEFAULT 'disabled',
  CHECK ((desired_tls_generation IS NULL) = (desired_tls_fingerprint IS NULL)),
  CHECK ((applied_tls_generation IS NULL) = (applied_tls_fingerprint IS NULL))
);

INSERT INTO ingress_listener_config (id) VALUES (1);

CREATE TABLE ingress_tls_material (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  generation INTEGER NOT NULL CHECK (generation > 0),
  fingerprint TEXT NOT NULL CHECK (length(fingerprint) > 0),
  approved_at INTEGER NOT NULL,
  approved_by TEXT NOT NULL CHECK (length(approved_by) BETWEEN 1 AND 128)
);
