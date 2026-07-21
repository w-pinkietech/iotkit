# IoTKit Edge Storage Profiles Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** IoTKit Edgeを、一つの正本だけを使う`embedded` (SQLite)と`postgres` (PostgreSQL)の両profileで動かし、停止・検証付きでSQLiteからPostgreSQLへ移行できるようにする。

**Architecture:** 現在の`store.Store`をdialect-awareな単一実装として維持し、上位のMQTT、application service、Console契約を変えない。SQL実行境界でplaceholderをdialectへ変換し、runtime queryは両DBで同じ意味を持たせる。schema、backup/restore、容量取得だけをprofile固有実装へ分離する。二重書込み、自動fallback、TimescaleDB依存は作らない。

**Tech Stack:** Go 1.25、`database/sql`、modernc SQLite、pgx PostgreSQL driver、PostgreSQL 17 Docker、Go test、Docker Compose。

## Global Constraints

- raw recordと`accepted_cursors`は同じtransactionでcommitし、commit成功前にackを返さない。
- `(edge_node_id, ledger_epoch, pub_seq)`、record hash、event/output identityをDB間で変えない。
- profileは導入時に固定し、接続失敗時に別DBへfallbackしない。
- credential、password、DSN内secretをargv、log、監査、Gitへ出さない。
- `embedded`と`postgres`は同じStorage適合testを通す。
- profile間移行はofflineのみとし、移行元と移行先を同時稼働させない。
- TimescaleDBは本計画へ含めない。

---

### Task 1: Storage profileとdialect実行境界

**Files:**
- Create: `iotkit-edge/internal/store/profile.go`
- Create: `iotkit-edge/internal/store/sql_database.go`
- Create: `iotkit-edge/internal/store/profile_test.go`
- Modify: `iotkit-edge/internal/store/store.go`
- Modify: `iotkit-edge/go.mod`

**Interfaces:**
- Produces: `type Profile string`、`ProfileEmbedded`、`ProfilePostgres`、`OpenOptions{Profile, SQLitePath, PostgresDSN, EdgeID}`、`OpenWithOptions(OpenOptions)`。
- Produces: dialect-aware `sqlDatabase` / `sqlTx` whose `ExecContext`、`QueryContext`、`QueryRowContext` rebind `?` to `$n` only for PostgreSQL.

- [ ] profile validation testを追加し、空profileは`embedded`へ正規化、PostgreSQLでSQLite pathを同時指定、未知profile、secretを含むerrorを拒否する。
- [ ] placeholder rebind testを追加し、文字列literalとquoted identifier内の`?`は変更せず、bind markerだけを連番化する。
- [ ] testを実行し、型と関数未実装で失敗することを確認する。
- [ ] `Profile`、`OpenOptions`、dialect-aware DB/Tx wrapperを最小実装する。
- [ ] SQLiteの既存`Open`/`OpenWithEdgeID`を`OpenWithOptions`の互換wrapperとして維持し、既存testを通す。
- [ ] focused testと`go test ./internal/store`を実行する。
- [ ] `feat(edge): add storage profile boundary`としてcommitする。

### Task 2: PostgreSQL schemaとcustody適合

**Files:**
- Create: `iotkit-edge/internal/store/postgres_migrations.go`
- Create: `iotkit-edge/internal/store/postgres_compat.go`
- Create: `iotkit-edge/internal/store/postgres_test.go`
- Create: `scripts/test-edge-postgres.sh`
- Modify: `iotkit-edge/internal/store/migrations.go`
- Modify: `iotkit-edge/internal/store/store.go`
- Modify: runtime SQL containing `INSERT OR IGNORE` under `iotkit-edge/internal/store/`

**Interfaces:**
- Consumes: `OpenWithOptions` and dialect-aware SQL wrapper.
- Produces: PostgreSQL schema version equal to current SQLite schema version and `OpenWithOptions(ProfilePostgres)`.

- [ ] Docker PostgreSQLを使うtest harnessを作り、一時databaseをtestごとに作成・破棄する。
- [ ] PostgreSQL open/schema identity testを追加し、未実装で失敗することを確認する。
- [ ] 現行SQLite migrationの最終schemaをPostgreSQL型へ写し、schema version tableとtransactional migrationを実装する。
- [ ] JSON抽出queryをDB非依存にし、hot queryがopaque JSONを同じ意味で読めるようにする。
- [ ] `INSERT OR IGNORE`を両DBで使える明示的`ON CONFLICT ... DO NOTHING`へ置換する。
- [ ] 共通custody testとして、activation済みbatch受理、exact replay、異内容conflict、gap、commit failureでackなし、cursor単調性を両profileへ実行する。
- [ ] account、semantic rule、projection、history、output outboxの代表journeyを両profileへ実行する。
- [ ] `scripts/test-edge-postgres.sh`とSQLite store suiteを通す。
- [ ] `feat(edge): add PostgreSQL storage profile`としてcommitする。

### Task 3: Profile別storage statusと安全なbackup/restore

**Files:**
- Create: `iotkit-edge/internal/store/postgres_backup.go`
- Create: `iotkit-edge/internal/store/postgres_storage_status.go`
- Modify: `iotkit-edge/internal/store/backup.go`
- Modify: `iotkit-edge/internal/store/backup_encrypted.go`
- Modify: `iotkit-edge/internal/store/storage_status.go`
- Modify: `iotkit-edge/internal/edgeapp/types.go`
- Test: `iotkit-edge/internal/store/backup_test.go`
- Test: `iotkit-edge/internal/store/storage_status_test.go`

**Interfaces:**
- Produces: profile-aware encrypted backup manifest with `storage_profile` and payload format.
- Produces: same logical storage status fields for both profiles, plus profile、growth rate、estimated days remaining、absolute reserve state.

- [ ] backup manifestがprofileを固定し、異なるprofileへの直接restoreを拒否するtestを追加する。
- [ ] PostgreSQLの一貫snapshotをcredential非露出で作成し、hash/cursor/schema/Edge IDを検証するtestを追加する。
- [ ] restoreを新しい空databaseへだけ許可し、session失効、recovery fence、cursor gap holdをSQLiteと同じ意味で実装する。
- [ ] PostgreSQL容量取得とSQLite DB/WAL合計取得のtestを追加する。
- [ ] 絶対空き容量、増加速度、推定残日数をstatusへ追加し、未配送outboxや未保護rawを削除しない。
- [ ] backup/restore focused suiteを両profileで通す。
- [ ] `feat(edge): support profile-aware backup and capacity status`としてcommitする。

### Task 4: Offline SQLite to PostgreSQL migration

**Files:**
- Create: `iotkit-edge/internal/store/profile_migration.go`
- Create: `iotkit-edge/internal/store/profile_migration_test.go`
- Modify: `iotkit-edge/cmd/iotkit-edge/main.go`
- Modify: `docs/edge-operations.md`

**Interfaces:**
- Produces CLI: `iotkit-edge storage migrate --from-sqlite PATH --to-postgres-config FILE --report FILE`。
- Produces a non-secret JSON verification report containing source/target profile、Edge ID、schema、table counts、cursor vector、pending outbox counts、content digest、completion state.

- [ ] 稼働中source、非empty target、Edge ID不一致、schema不一致、件数/hash/cursor不一致を拒否するtestを追加する。
- [ ] 全tableをforeign-key-safe orderでcopyし、sequenceを最大IDの次へ合わせるtransactional importerを実装する。
- [ ] import後にraw identity/hash、cursor、semantic observation、account/audit、pending outboxを比較し、不一致ならtargetを未完成のまま残して切替を禁止する。
- [ ] CLIがsecretをargv/report/logへ出さず、owner-only reportを原子的に作るtestを追加する。
- [ ] SQLite fixtureからPostgreSQLへ移行し、同じhistory/Console read modelとpending outputを読める統合testを通す。
- [ ] `feat(edge): add verified SQLite to PostgreSQL migration`としてcommitする。

### Task 5: 導入profileとfail-closed起動

**Files:**
- Create: `deploy/compose.edge-postgres.yaml`
- Modify: `deploy/compose.edge.yaml`
- Modify: `scripts/bootstrap-edge.sh`
- Modify: `scripts/test-edge-bootstrap.sh`
- Modify: `iotkit-edge/cmd/iotkit-edge/main.go`
- Modify: `README.md`
- Modify: `docs/edge-operations.md`

**Interfaces:**
- Produces CLI flags: `--storage-profile embedded|postgres`、`--postgres-config FILE`; DSN/passwordはowner-only fileから読む。
- Produces generated deployment metadata recording the immutable selected profile.

- [ ] bootstrap profile選択、owner-only PostgreSQL config、未知profile、既存profile不一致のtestを追加する。
- [ ] embedded Composeを現在互換のまま維持し、PostgreSQL Composeはdigest-pinned image、healthcheck、resource limits、local-only DB exposure、persistent volumeを持たせる。
- [ ] Edge起動時にprofile metadataとflagを照合し、不一致・DB接続失敗・durability設定不足でfail closedする。
- [ ] Consoleのシステム画面へprofileと検証済みcapacity envelopeを表示する。
- [ ] clean bootstrap testを両profileで通す。
- [ ] `feat(edge): add embedded and PostgreSQL deployment profiles`としてcommitする。

### Task 6: Capacity gateと最終検証

**Files:**
- Create: `scripts/test-edge-capacity.sh`
- Create: `docs/edge-capacity.md`
- Modify: `scripts/verify.sh`
- Modify: `README.md`
- Modify: `docs/redesign/decisions/D3-process-and-wave-decisions.md`

**Interfaces:**
- Produces reproducible load report for profile、hardware、Edge Nodes、sensors、records/s、payload bytes、rules、retention、query/backup workload.

- [ ] 疑似Edge Node複数台のbatch generatorを作り、通常受信とburstを再現する。
- [ ] 受信中に31日graph相当query、100k CSV、semantic catch-up、output outage、backup、restartを重ねる。
- [ ] accepted-through p99、backlog、projection/outbox lag、DB/WAL bytes、CPU/RAM、query/backup時間をJSONへ出す。
- [ ] 数値未計測のprofileを「対応済み」と表示しないfail-closed capacity metadataを実装する。
- [ ] SQLite、PostgreSQLのfocused tests、Console browser journey、Broker/output、backup/migration gateを実行する。
- [ ] Rust製品動作は変更しないためRust全体testは省略し、`go test ./...`、frontend、Docker integration、`git diff --check`を実行する。
- [ ] `test(edge): add storage profile capacity gate`としてcommitする。

