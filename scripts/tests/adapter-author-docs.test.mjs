import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";

const repoRoot = path.resolve(import.meta.dirname, "../..");
const contracts = ["en", "ja"].map((locale) => ({
  locale,
  source: fs.readFileSync(
    path.join(repoRoot, "docs", "okf", locale, "contracts", "output-adapter-v1.md"),
    "utf8",
  ),
}));

test("current output adapter contracts use the Rust public API", () => {
  for (const { locale, source } of contracts) {
    assert.doesNotMatch(source, /```go\b/, `${locale} still contains a Go API block`);
    assert.doesNotMatch(source, /\bModeDescriptor\b/);
    assert.doesNotMatch(source, /\bMQTTPublication\b/);
    assert.doesNotMatch(source, /\bErrInvalidDescriptor\b/);
    assert.match(source, /pub trait OutputAdapter/);
    assert.match(source, /MqttPublication/);
    assert.match(source, /AdapterError::InvalidDescriptor/);
  }
});

test("current output adapter contracts point authors to compiled Rust examples", () => {
  for (const { locale, source } of contracts) {
    assert.match(source, /edge\/output-adapters\/api/);
    assert.match(source, /edge\/output-adapters\/example/);
    assert.match(source, /edge\/output-adapters\/testkit/);
    assert.match(source, /cargo test -p iotkit-output-adapter-example/);
  }
});
