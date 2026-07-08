CREATE TABLE admin_credential (
  id INTEGER PRIMARY KEY CHECK (id = 1),   -- 単一行
  passphrase_hash TEXT NOT NULL,           -- argon2id PHC 文字列
  set_at INTEGER NOT NULL,                 -- unix ms
  updated_at INTEGER NOT NULL
);
CREATE TABLE operator_tokens (
  token_id TEXT PRIMARY KEY,               -- "tok_" + base64url(16byte乱数)=22文字（表示・監査用の公開ID）
  name TEXT NOT NULL,
  token_hash BLOB NOT NULL,                -- SHA-256(トークン平文)
  kind TEXT NOT NULL CHECK (kind IN ('human','ai')),
  tier_ceiling TEXT NOT NULL CHECK (tier_ceiling IN ('read_only','routine','daily','construction')),
  is_session INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  expires_at INTEGER,                      -- NULL=無期限（AI ハーネス用長命）
  revoked_at INTEGER,
  last_used_at INTEGER,
  CHECK (kind != 'ai' OR tier_ceiling IN ('read_only','routine'))
);
CREATE UNIQUE INDEX idx_operator_tokens_hash ON operator_tokens(token_hash);
