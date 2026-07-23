# Rust IoTKit Edge CLI Parity Design

Status: Approved for implementation on 2026-07-24

## 1. Goal and scope

The Rust `iotkit-edge` binary replaces the remaining Go operator commands:

- `storage migrate`
- `query`
- `mapping-set`
- `mapping-deactivate`
- `mapping-list`
- `route-add`
- `route-list`
- `semantic-query`

The Go binary in `edge/cmd/iotkit-edge/main.go` is the black-box oracle for
flags, required-argument ordering, pretty JSON fields, stdout/stderr separation,
and exit status. Current OKF product, storage, semantic, and Output Adapter
contracts remain authoritative where the old implementation differs from the
Rust model.

The compatibility commands do not restore the Go semantic or MQTT route tables.
The current Rust semantic rule, observation, export profile, binding, route,
and outbox schema remains the only source of truth.

## 2. Boundaries

Clap parses bounded transport values and dispatches one typed application
operation. CLI code does not issue SQL, select an SQL dialect, or switch on an
Output Adapter identifier.

New application units are:

- `CliQueries`, for raw and semantic compatibility reads;
- `LegacyMappings`, for the reversible production-pulse view over semantic
  rules;
- `OutputRoutes`, for provider-neutral exact-route creation and route status;
- `StorageMigration`, for the offline SQLite-to-PostgreSQL operation.

Storage owns backend-specific reads, transactions, migration copy mechanics,
locks, and verification. Application services validate intent and convert
typed results into compatibility result types. Composition supplies the generic
MQTT JSON Adapter registration to `OutputRoutes`; storage receives a validated
registration and versioned route configuration without provider-specific
branches.

All mutations append the existing local CLI audit actor and operation summary
in the same transaction as the mutation. Compatibility aliases are computed,
not separately persisted.

## 3. Go-compatible command surface

Every command accepts the old `--db` flag and defaults to `edge.db`. Read and
mutation commands additionally accept the common Rust
`--storage-profile`, `--postgres-config`, and `--storage-metadata` flags so the
same operation works against either supported authoritative profile.

Limits are inclusive integers from 1 through 10,000. JSON is indented by two
spaces, terminated by one newline, and is the only stdout content. Errors and
Clap usage go only to stderr and return a non-zero process exit. Successful
commands return zero.

Validation that does not need storage happens before opening or creating a
database. This preserves the Go behavior for missing mapping flags, unsupported
meaning or trigger values, invalid active values, invalid MQTT topics, and
invalid limits.

`query` returns:

```json
[
  {
    "edge_node_id": "edge-node-01",
    "ledger_epoch": "epoch-01",
    "pub_seq": 1,
    "publication_id": "edge-node-01:epoch-01:1:1",
    "record": {},
    "received_at": 1720000000123
  }
]
```

Ordering is `received_at DESC, edge_node_id, ledger_epoch, pub_seq DESC`.

## 4. Legacy mapping view over semantic rules

Only the old valid meaning, `production_pulse`, is accepted. It maps to an
active Rust semantic rule with display name `production_pulse` and kind
`cumulative_counter`:

| Go field | Rust rule field |
|---|---|
| `active_sample` | `TriggerMode::OnNotification` |
| `active_edge` | `TriggerMode::OnTransition` |
| `active_value=1` | `DetectorMode::BooleanHighActive` |
| `active_value=0` | `DetectorMode::BooleanLowActive` |
| mapping revision | semantic rule revision |
| mapping active | semantic rule active |

Both detector thresholds and debounce values are zero. The mapping therefore
counts each active sample or each inactive-to-active transition exactly as the
Go evaluator did.

The compatibility mapping ID is reversible:

```text
Rust rule UUID  550e8400-e29b-41d4-a716-446655440000
CLI mapping ID  sm-550e8400e29b41d4a716446655440000
```

Only UUID-backed rules created through the compatibility operation and having
the exact display name, kind, and compatible detector/trigger combination
appear in `mapping-list`. This prevents an unrelated Console rule from being
silently treated as a legacy mapping.

`mapping-set` finds that compatibility rule by `(edge_node_id, series_key)`.
It creates the first rule or revises the same stable rule, using the existing
future-only accepted-cursor boundary. `mapping-deactivate` retires only that
rule and drains its output bindings without deleting observations or pending
outbox rows.

`mapping-list` reads `semantic_rule_revisions` and returns every revision in
rule/revision order. Only the current revision reports `active: true`; retired
and superseded revisions report false. `created_at` comes from the immutable
revision row. Mutation responses preserve the Go mapping JSON fields exactly.

## 5. Semantic compatibility query

`semantic-query` returns observations produced by compatibility mapping rules.
It does not project data and does not read deprecated Go tables.

The compatibility fields are:

| Go event field | Rust source |
|---|---|
| `event_id` | `observation_id` |
| `mapping_id` | reversible alias of `rule_id` |
| `mapping_revision` | observation `revision` |
| `event_sequence` | observation `sequence` |
| `meaning` | constant `production_pulse` |
| `edge_node_id` | observation source |
| `ledger_epoch` | observation source |
| `source_pub_seq` | observation source |
| `source_series_key` | semantic signal `series_key` |
| `occurred_at` | `observed_at` |
| `created_at` | observation `created_at` |

Ordering is semantic observation row order with the requested limit.

## 6. Provider-neutral output route fan-out

The current output model is extended so one active binding can have multiple
routes. This is a provider-neutral capability required by the Output Adapter
contract: one semantic observation may be transformed to multiple exact topics
by distinct routes.

`output_routes.binding_id` is no longer unique. Every route gains
`start_after_observation_row_id`. Route candidate queries require both the
existing binding cursor boundary and:

```text
semantic_observation.observation_row_id > route.start_after_observation_row_id
```

Thus a route added to an already-active binding is future-only and never
backfills older observations. Existing routes receive a zero route boundary and
retain their existing binding boundary behavior.

The append-only SQLite migration rebuilds `output_routes`,
`output_outbox`, and `output_route_attempts` in foreign-key-safe order while
preserving every row. The PostgreSQL migration drops the old unique constraint
and adds the boundary column. Both migrations keep route IDs, outbox IDs,
attempts, lifecycle state, claims, and PUBACK state unchanged.

The provider-neutral application request contains:

- semantic rule ID;
- an `OutputAdapterRegistration`;
- adapter mode;
- versioned non-secret route configuration;
- timestamp.

Storage validates the config through the supplied Adapter before writing. It
finds or creates the Adapter's live export profile and the rule binding, then
adds an idempotent exact route. The first compatibility route can create a
profile and binding without generating an unintended default route. Later
routes fan out from the same binding. Existing Console-created profiles and
bindings remain usable.

The CLI composition selects the registered `iotkit.mqtt-json.v1` Adapter and
constructs schema-version-1 config with the requested exact topic. Topic
validation rejects blank topics, leading or trailing slash, NUL, `+`, and `#`
before storage opens. Core application and storage code contain no adapter-ID
switch.

Route aliases are reversible from Rust `route_<32 hex>` IDs to Go-compatible
`mr-<32 hex>` IDs. `route-add` returns:

- route alias;
- mapping alias;
- exact topic;
- QoS 1;
- `start_after_event_row_id`, equal to the stored observation boundary;
- active state;
- creation time.

`route-list` derives topic from the non-secret versioned config through the
composition-supplied compatibility codec. It adds pending count, published
count, and oldest pending creation time from the current durable outbox. It
lists only routes attached to compatibility mapping rules and the selected
generic Adapter. No credential or payload is returned.

## 7. Offline Rust SQLite-to-PostgreSQL migration

`storage migrate` accepts only:

```text
--from-sqlite PATH
--to-postgres-config FILE
--report FILE
```

The source must be an existing regular file. The PostgreSQL configuration is an
owner-only regular JSON file containing only a non-empty `dsn`. The report
destination must not exist.

The operation proceeds in this order:

1. acquire the canonical exclusive SQLite deployment lock;
2. verify free staging capacity is at least the SQLite database size;
3. create an owner-only staging directory and consistent `VACUUM INTO`
   snapshot, sync it, and run `PRAGMA quick_check`;
4. require the exact current Rust SQLx migration set and exact product table and
   column layout; a Go-era database, failed/ahead migration, unknown table, or
   unknown column is rejected;
5. connect to PostgreSQL, acquire the normal IoTKit advisory owner lock, run the
   current Rust migrations, and require every product table to be empty;
6. start one PostgreSQL transaction and copy every product table in explicit
   foreign-key order, preserving IDs, byte fields, JSON values, timestamps,
   revision boundaries, claims, outbox state, and audit data;
7. reset PostgreSQL identity sequences used by imported rows;
8. compute source and target per-table row counts and a canonical typed row
   digest ordered by primary key;
9. compare Edge identity, accepted cursor vector, every table count, and the
   combined canonical digest before commit;
10. commit, re-read the same verification view from PostgreSQL, and require it
    to match the pre-commit report;
11. write the completed report atomically with mode `0600`, `sync_all`, and
    no-clobber hard-link publication.

SQLite bytes/blob and PostgreSQL `bytea`, SQLite JSON bytes and PostgreSQL
`jsonb`, booleans, integers, reals, text, and nulls have explicit canonical
encodings. The table metadata is a closed Rust catalog; neither `sqlx::Any` nor
an inferred arbitrary SQL identifier is used.

The report matches the operator contract:

```json
{
  "source_profile": "embedded",
  "target_profile": "postgres",
  "edge_id": "edge-0123456789abcdef0123456789abcdef",
  "schema_version": 6,
  "table_counts": {},
  "cursors": [],
  "content_digest": "64 lowercase hexadecimal characters",
  "completed": true
}
```

Any copy or verification failure rolls back the PostgreSQL transaction and
does not create a report. A non-empty or live destination is rejected. The
source database is never changed or removed.

## 8. Failure and security behavior

- DSNs and password-bearing configuration never implement `Debug` and never
  appear in stdout, stderr, logs, report content, audit summaries, or fixtures.
- Route config is non-secret and Adapter-validated.
- Invalid CLI input cannot create a database.
- Read commands never mutate semantic, output, or audit state.
- Mutation audit failure rolls back the mutation.
- Migration failure leaves the source authoritative, target unusable until
  recreated, and report absent.
- No command silently falls back between storage profiles.

## 9. Verification

Tests first establish the Go oracle's accepted flags, validation order, JSON
field names, formatting, stdout/stderr, and exit status.

Shared application/storage contract vectors run against SQLite and real
PostgreSQL for:

- mapping create, revision, list, deactivate, and future-only projection;
- mapping alias round trip;
- one and multiple exact-topic route fan-out;
- route boundary, Adapter validation, output outbox, and PUBACK counts;
- raw and semantic query ordering and limits.

The real migration gate proves:

- complete fresh Rust SQLite data, including identity, activation, raw,
  semantic, auth, audit, route, pending/published outbox, backup, and restore
  rows, reaches PostgreSQL unchanged;
- report mode, no-clobber behavior, cursor/count/digest match;
- Go-era source rejection;
- non-empty and live target rejection;
- injected copy or verification failure rolls back and produces no report.

Final verification runs Rust formatting, full IoTKit Edge tests, Clippy with
warnings denied, source/layer checks, the real PostgreSQL gate, and the
battle-tested review selector.
