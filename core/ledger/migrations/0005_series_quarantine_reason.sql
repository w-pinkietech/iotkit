-- D6決定6: 未知キー検疫series実体化にquarantine_reasonを付す(unknown_key | undeclared_channel)
ALTER TABLE series ADD COLUMN quarantine_reason TEXT;
