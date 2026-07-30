CREATE TABLE edge_node_recovery_activation (
  singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
  state TEXT NOT NULL CHECK (state IN ('applied', 'completed')),
  request_json TEXT NOT NULL CHECK (json_valid(request_json)),
  result_json TEXT NOT NULL CHECK (json_valid(result_json)),
  completion_json TEXT CHECK (completion_json IS NULL OR json_valid(completion_json)),
  applied_at_ms INTEGER NOT NULL CHECK (applied_at_ms >= 0),
  completed_at_ms INTEGER CHECK (completed_at_ms IS NULL OR completed_at_ms >= 0),
  CHECK (
    (state = 'applied' AND completion_json IS NULL AND completed_at_ms IS NULL)
    OR
    (state = 'completed' AND completion_json IS NOT NULL AND completed_at_ms IS NOT NULL)
  )
);

CREATE TRIGGER edge_node_recovery_activation_forward_only
BEFORE UPDATE ON edge_node_recovery_activation
WHEN OLD.state <> 'applied'
  OR NEW.state <> 'completed'
  OR NEW.singleton <> OLD.singleton
  OR NEW.request_json <> OLD.request_json
  OR NEW.result_json <> OLD.result_json
  OR NEW.applied_at_ms <> OLD.applied_at_ms
  OR NEW.completion_json IS NULL
  OR NEW.completed_at_ms IS NULL
BEGIN
  SELECT RAISE(ABORT, 'recovery activation transition is not allowed');
END;

CREATE TRIGGER edge_node_recovery_activation_insert_state
BEFORE INSERT ON edge_node_recovery_activation
WHEN NEW.state <> 'applied'
  OR NEW.completion_json IS NOT NULL
  OR NEW.completed_at_ms IS NOT NULL
BEGIN
  SELECT RAISE(ABORT, 'recovery activation must begin applied');
END;

CREATE TRIGGER edge_node_recovery_activation_immutable_delete
BEFORE DELETE ON edge_node_recovery_activation
BEGIN
  SELECT RAISE(ABORT, 'recovery activation is immutable');
END;
