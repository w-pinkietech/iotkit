# Historical corpus selection policy v1

Status: frozen before any evaluator capture for [#214](https://github.com/w-pinkietech/iotkit/issues/214).

This recorder artifact selects a small historical corpus for the experimental,
report-only pure-refactoring evaluator. It grants no status, approval,
threshold, merge, or product authority.

## Fixed population and capture rule

The repository cutoff is `master` at
`7070e73577763e893e9f23bd8456ace3799ebfd0`. Every source and merge reference
below was verified as a merged pull-request association before this policy was
frozen. A source diff is the exact LF-normalized output of:

```text
git diff --no-ext-diff --unified=3 <source_parent_sha> <source_commit_sha> -- <pathspec...>
```

When the complete source-commit diff has at most 120 changed lines, its
pathspec lists every changed path and `complete_source_commit` is recorded.
Otherwise `path_family` records every hunk for the declared production and
closest test/contract family; no line or hunk is hand-picked. The raw,
normalized source diff is hashed before sanitization.

The model diff normalizes line endings to LF and removes only Git `index`
object-ID lines. Recorder-only provenance stores each removed line verbatim
with its one-based raw line position, so validation can deterministically
restore the raw diff, verify its SHA-256, and verify that re-sanitizing it
reproduces the model diff. Pull-request number, commit identifier, title, and
label metadata are never included in the model diff. A candidate needing
content redaction is rejected rather than edited. The model sees exactly
`case_id` and the resulting diff.

The fixed population has four mechanically structural positives, one case for
each hard exclusion, and one insufficient-evidence boundary. Labels and kinds
below were fixed before any run. No more than two cases come from one PR. The
opaque-ID order must retain both labels in both halves; if it does not, stop
and report that failure rather than changing selection by hash or model outcome.

## Frozen candidates

| PR | source commit | selection | answer key | kind |
| --- | --- | --- | --- | --- |
| 127 | `95b6afb9cf9e3d555357b6927103b1346a0bb64e` | complete `edge/src/composition/runtime_config.rs` | structural_only / proven | positive |
| 60 | `c0826a00619164198232e3a452b424b505091586` | complete `core/collector/src/actor.rs`, `core/ledger/src/lib.rs`, `core/timeseries/src/lib.rs`, `iotkit-edge/src/api/tls.rs`, `iotkit-edge/src/config.rs`, `iotkit-edge/src/mqtt_publish_task.rs`, `iotkit-edge/tests/api_basic.rs`, `iotkit-ingest-http/tests/e2e.rs` | structural_only / proven | positive |
| 84 | `a8dd790a5d778130bcacb6b64d8529cf50ec514d` | complete `edge/src/backup/mod.rs`, `edge/src/backup/sqlite.rs`, `edge/src/diagnostics/mod.rs`, `edge/tests/unit/backup_sqlite_tests.rs`, `edge/tests/unit/backup_tests.rs`, `edge/tests/unit/diagnostics_tests.rs` | structural_only / proven | positive |
| 84 | `0069e79457d35bdb00c29bfe326e55a38a081033` | complete `edge/src/mqtt/ingest/runtime.rs`, `edge/tests/unit/mqtt_ingest_runtime_tests.rs` | structural_only / proven | positive |
| 123 | `2a9dd753e72222de0516c7a70ece48bcff180473` | complete certificate-hostname change | auth_secrets / not_proven | negative |
| 192 | `303db799cac7ce5a62a9086803dd7b3a3410f99f` | MQTT publish, wire, and unit/fixture family: `edge-node/apps/node/src/mqtt_publish_task.rs`, `edge-node/apps/node/tests/unit/mqtt_publish_task_tests.rs`, `edge-node/core/publish/src/wire.rs`, `edge-node/core/publish/tests/egress_v1_fixtures.rs` | custody_data_loss / not_proven | negative |
| 38 | `6ef5c582b122770f801f20a6c94a338b2d9424bc` | migration production family: `core/storage/migrations/0001_init.sql`, `core/storage/src/lib.rs`, `core/storage/src/migrate.rs` | database_migration / not_proven | negative |
| 122 | `c3b760da7e957f978b3d1400ebd564084990625b` | complete paired recovery-contract change | backup_restore / not_proven | negative |
| 209 | `c5ed685935ac19744edd9517545bad779528081c` | shutdown production/unit family: `edge-node/apps/node/src/main.rs`, `edge-node/apps/node/tests/unit/main_tests.rs` | concurrency_timing / not_proven | negative |
| 127 | `f6d498c4368a3c782edb7a25b4162e344be0728f` | complete trial configuration family | configuration_deployment / not_proven | adversarial |
| 206 | `088f978ece48d149415bffa7182e0c9a7b739a93` | complete release-version/dependency change | dependencies / not_proven | negative |
| 184 | `5fb884965b8e74606413469604f953c95ab03aa4` | frontend source, generated static, and closest unit family: `edge/frontend/src/live.ts`, `edge/frontend/static/console.js`, `edge/frontend/tests/unit/live.test.ts` | generated_artifacts / not_proven | adversarial |
| 60 | `cf5dc6e3fefa941e8a72f2df691d6f736d36cd7f` | complete MQTT identity contract family | public_wire_api_contract / not_proven | negative |
| 121 | `91c3164b01621916715764ea3a7c03e59cdbb379` | paired field-guide contract family: `docs/okf/en/operations/edge-node-hardware-recovery.md`, `docs/okf/ja/operations/edge-node-hardware-recovery.md` | product_documentation_authority / not_proven | negative |
| 204 | `621405dbd5b0aaf501e905edc0d69e808ff3e348` | complete navigation/template contract change | operator_visible_behavior / not_proven | negative |
| 65 | `31768781f70bbd7417980cca53f5e650fa1623d7` | complete test assertion/process-checkbox merge diff | insufficient_evidence / not_proven | adversarial |

The PR 65 source is its associated squash/merge commit. Its one-parent source
diff is retained with that parent; it is not substituted with a later or
unrelated commit.

## Exclusions

- Do not call a model or GitHub client from the evaluator or CI.
- Do not add an unreviewed capture, report, recommendation, or comparison to
  this policy phase.
- Do not infer behavioral equivalence, release readiness, or merge authority
  from this corpus or any future score.
