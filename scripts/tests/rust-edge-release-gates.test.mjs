import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const scripts = [
  "verify.sh",
  "test-edge-capacity.sh",
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
