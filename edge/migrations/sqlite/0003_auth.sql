CREATE TABLE edge_accounts (
    account_ref TEXT PRIMARY KEY,
    login_id TEXT NOT NULL,
    login_id_normalized TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL CHECK(length(display_name) BETWEEN 1 AND 128),
    password_phc TEXT NOT NULL,
    role TEXT NOT NULL CHECK(role IN ('viewer', 'admin', 'system_admin')),
    state TEXT NOT NULL CHECK(state IN ('active', 'disabled')),
    must_change_password INTEGER NOT NULL CHECK(must_change_password IN (0, 1)),
    revision INTEGER NOT NULL CHECK(revision > 0),
    created_at INTEGER NOT NULL CHECK(created_at >= 0),
    updated_at INTEGER NOT NULL CHECK(updated_at >= 0),
    disabled_at INTEGER
);

CREATE TABLE edge_sessions (
    session_ref TEXT PRIMARY KEY,
    token_sha256 BLOB NOT NULL UNIQUE CHECK(length(token_sha256) = 32),
    csrf_sha256 BLOB NOT NULL CHECK(length(csrf_sha256) = 32),
    account_ref TEXT NOT NULL REFERENCES edge_accounts(account_ref),
    issued_at INTEGER NOT NULL CHECK(issued_at >= 0),
    last_seen_at INTEGER NOT NULL CHECK(last_seen_at >= issued_at),
    idle_expires_at INTEGER NOT NULL CHECK(idle_expires_at > issued_at),
    absolute_expires_at INTEGER NOT NULL CHECK(absolute_expires_at > issued_at),
    revoked_at INTEGER
);

CREATE INDEX idx_edge_sessions_account_active
ON edge_sessions(account_ref, revoked_at);

CREATE TABLE audit_events (
    audit_row_id INTEGER PRIMARY KEY AUTOINCREMENT,
    occurred_at INTEGER NOT NULL CHECK(occurred_at >= 0),
    actor_class TEXT NOT NULL CHECK(actor_class IN ('account', 'local_cli', 'system')),
    actor_ref TEXT NOT NULL,
    actor_login_id TEXT,
    actor_display_name TEXT,
    operation TEXT NOT NULL,
    resource_ref TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK(outcome IN ('success', 'failure')),
    summary_json BLOB NOT NULL CHECK(json_valid(summary_json))
);

CREATE INDEX idx_audit_events_recent
ON audit_events(audit_row_id DESC);
