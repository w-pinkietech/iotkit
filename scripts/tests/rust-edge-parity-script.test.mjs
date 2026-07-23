import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const source = readFileSync(
  new URL("../test-edge-parity.sh", import.meta.url),
  "utf8",
);

test("the final parity gate runs both implementations and records evidence", () => {
  assert.doesNotMatch(source, /full parity is unavailable/);
  assert.match(source, /go test \.\/\.\.\./);
  assert.match(source, /cargo test .*iotkit-edge/);
  assert.match(source, /test-edge-console-e2e\.sh/);
  assert.match(source, /test-edge-output\.sh/);
  assert.match(source, /IOTKIT_EDGE_PARITY_REPORT_DIR/);
  assert.match(source, /result\.json/);
});
