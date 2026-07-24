ALTER TABLE output_routes
    DROP CONSTRAINT output_routes_binding_id_key;

ALTER TABLE output_routes
    ADD COLUMN start_after_observation_row_id BIGINT NOT NULL DEFAULT 0
        CHECK(start_after_observation_row_id >= 0);

CREATE INDEX ix_output_routes_binding
    ON output_routes(binding_id, created_at, route_id);
