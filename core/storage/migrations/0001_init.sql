-- Baseline migration for iotkit-core-storage.
-- Application tables are added by downstream issues (#22, #23).

-- Private cutover marker. Because migration v1 is already recorded in pre-release databases,
-- changing its body marks only databases first initialized by the Edge Node-named code.
CREATE TABLE _iotkit_edge_format (
    singleton      INTEGER NOT NULL PRIMARY KEY CHECK (singleton = 1),
    format_version INTEGER NOT NULL CHECK (format_version = 1)
) WITHOUT ROWID;

INSERT INTO _iotkit_edge_format (singleton, format_version) VALUES (1, 1);
