CREATE INDEX ix_semantic_observation_rule_observed_at_row
    ON semantic_observations(rule_id,observed_at,observation_row_id);
