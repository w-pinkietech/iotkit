import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const scripts = [
  "verify.sh",
  "test-edge-capacity.sh",
  "test-edge-node-sigterm.sh",
  "test-edge-postgres.sh",
  "test-edge-output.sh",
  "test-edge-mqtt.sh",
  "test-edge-host-release-gate.sh",
  "test-broker-cert-pebble.sh",
];

const sources = Object.fromEntries(
  scripts.map((name) => [
    name,
    readFileSync(new URL(`../${name}`, import.meta.url), "utf8"),
  ]),
);
const bootstrapGate = readFileSync(
  new URL("../test-edge-bootstrap.sh", import.meta.url),
  "utf8",
);

test("active Rust Edge release gates do not require the Go toolchain", () => {
  for (const [name, source] of Object.entries(sources)) {
    assert.doesNotMatch(
      source,
      /\bgo (?:test|install)\b|GOCACHE|GOMODCACHE|GOTMPDIR|golang:/,
      name,
    );
  }
});

test("capacity gate emits and validates both existing evidence profiles", () => {
  const source = sources["test-edge-capacity.sh"];
  assert.match(source, /capacity_regression/);
  assert.match(source, /embedded\.json/);
  assert.match(source, /postgres\.json/);
  assert.match(source, /regression_smoke_passed/);
  assert.match(source, /IOTKIT_CAPACITY_REPORT/);
});

test("MQTT and output gates retain real broker and outage coverage", () => {
  assert.match(sources["verify.sh"], /test-edge-output\.sh/);
  assert.match(sources["test-edge-mqtt.sh"], /test-rust-edge-custody\.sh/);
  assert.match(sources["test-edge-mqtt.sh"], /test-rust-edge-runtime\.sh/);
  assert.match(
    sources["test-edge-output.sh"],
    /actual_mosquitto_outage_retries_same_durable_export_until_puback/,
  );
  assert.match(sources["test-edge-output.sh"], /docker stop/);
  assert.match(sources["test-edge-output.sh"], /docker start/);
});

test("Pebble gate uses the pinned official lego image", () => {
  const source = sources["test-broker-cert-pebble.sh"];
  assert.match(
    source,
    /goacme\/lego:v4\.35\.2@sha256:ae124a405844759b201b31efbd7a0ba302dbd16e86f2fb177c4b6db8bdc782c8/,
  );
  assert.match(source, /docker (?:run|create)/);
});

test("certificate renewal exercises the production renew command", () => {
  assert.match(
    sources["test-edge-host-release-gate.sh"],
    /test-certificate-hostname\.sh/,
  );
  assert.match(
    sources["test-broker-cert-pebble.sh"],
    /iotkit-broker-cert"\s+renew/,
  );
  assert.match(
    sources["test-broker-cert-pebble.sh"],
    /serial_before[\s\S]+serial_after/,
  );
  const certificateCommand = readFileSync(
    new URL("../iotkit-broker-cert", import.meta.url),
    "utf8",
  );
  assert.match(certificateCommand, /"\$\{challenge_args\[@\]\}"\s+renew\b/);
  assert.doesNotMatch(certificateCommand, /"\$\{challenge_args\[@\]\}"\s+run\b/);
});

test("resilience gate supplies identity and checks both SQLite databases", () => {
  const compose = readFileSync(
    new URL("../../compose.dev.yaml", import.meta.url),
    "utf8",
  );
  const resilience = readFileSync(
    new URL("../test-edge-resilience.sh", import.meta.url),
    "utf8",
  );
  assert.match(compose, /--edge-id[\s\S]+IOTKIT_EDGE_ID/);
  assert.match(resilience, /export IOTKIT_EDGE_ID=/);
  assert.match(resilience, /edge_node_check=/);
  assert.match(resilience, /central_edge_check=/);
});

test("SIGTERM gate runs the Edge Node as the container primary process", () => {
  const compose = readFileSync(
    new URL("../../deploy/compose.edge-node-sigterm.yaml", import.meta.url),
    "utf8",
  );
  const trialCompose = readFileSync(
    new URL("../../deploy/compose.trial.yaml", import.meta.url),
    "utf8",
  );
  const source = sources["test-edge-node-sigterm.sh"];
  assert.match(compose, /stop_grace_period: 15s/);
  assert.match(compose, /entrypoint: \["\/usr\/local\/bin\/iotkit-edge-node"\]/);
  assert.match(trialCompose, /edge-node:[\s\S]*?stop_grace_period: 15s/);
  assert.match(source, /docker kill --signal SIGTERM/);
  assert.match(source, /mkfifo/);
  assert.match(source, /early-start/);
  assert.match(source, /State\.ExitCode/);
  assert.match(source, /PRAGMA quick_check/);
  assert.match(source, /publication_log/);
});

test("PostgreSQL gate covers upgrades, profile migration, and recovery hold", () => {
  const source = sources["test-edge-postgres.sh"];
  assert.match(source, /schema_upgrade_contract/);
  assert.match(
    source,
    /postgres_migration_copies_and_verifies_a_fresh_rust_schema_when_configured/,
  );
  assert.match(
    source,
    /postgres_migration_failure_rolls_back_every_copied_row_when_configured/,
  );
  assert.match(
    source,
    /postgres_restored_gap_requires_audited_archive_loss_acceptance/,
  );
});

test("host release gate saves the real Edge Node recovery drill evidence", () => {
  const source = sources["test-edge-host-release-gate.sh"];
  assert.match(source, /IOTKIT_TEST_RECOVERY_DRILL=1/);
  assert.match(source, /IOTKIT_RECOVERY_EVIDENCE_DIR/);
  assert.match(source, /edge-node-recovery/);
  assert.match(bootstrapGate, /passphrase reset/);
  assert.match(bootstrapGate, /ownership_reestablished/);
  assert.match(bootstrapGate, /post_recovery_backup_verified/);
});

test("real output outage covers generic and Pinikiet exports", () => {
  const source = sources["test-edge-output.sh"];
  const rustTest = readFileSync(
    new URL("../../edge/tests/output_puback.rs", import.meta.url),
    "utf8",
  );
  assert.match(source, /pinikiet\/v1\/sources\/\+\/sensors\/\+\/observations/);
  assert.match(source, /pinikiet\/v1\/sources\/\+\/status/);
  assert.match(rustTest, /pinikiet\.mqtt\.v1/);
  assert.match(rustTest, /generic_export_id/);
  assert.match(rustTest, /pinikiet_export_id/);
});

test("bootstrap creates the first administrator before starting the Edge owner", () => {
  const accountBootstrap = bootstrapGate.indexOf("edge account bootstrap");
  const edgeStart = bootstrapGate.indexOf(
    '"${compose[@]}" up --build --detach',
  );
  assert.notEqual(accountBootstrap, -1);
  assert.notEqual(edgeStart, -1);
  assert.ok(
    accountBootstrap < edgeStart,
    "the one-shot bootstrap must own storage before the long-running Edge starts",
  );
});

test("bootstrap expects the current API state for discovered Edge Nodes", () => {
  assert.match(bootstrapGate, /\.state == "needs-setup"/);
  assert.match(bootstrapGate, /\.state == "configured"/);
  assert.doesNotMatch(bootstrapGate, /\.state == "discovered"/);
  assert.doesNotMatch(bootstrapGate, /\.state == "active"/);
});

test("running bootstrap verifies custody through the authenticated API", () => {
  assert.match(
    bootstrapGate,
    /api\/v1\/history\?from=\$history_from&to=\$history_to&limit=10/,
  );
  assert.match(bootstrapGate, /history_from=\$\(\(history_to - 60000\)\)/);
  const retryLoop = bootstrapGate.indexOf("for _ in $(seq 1 60); do");
  const historyWindow = bootstrapGate.indexOf(
    "history_to=$(date +%s%3N)",
    retryLoop,
  );
  const historyRequest = bootstrapGate.indexOf(
    "api/v1/history?from=$history_from&to=$history_to&limit=10",
    historyWindow,
  );
  assert.ok(retryLoop < historyWindow && historyWindow < historyRequest);
  assert.doesNotMatch(bootstrapGate, /exec -T edge[\s\S]+iotkit-edge query/);
});
