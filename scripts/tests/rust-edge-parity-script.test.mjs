import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const source = readFileSync(
  new URL("../test-edge-parity.sh", import.meta.url),
  "utf8",
);

test("the post-cutover parity gate runs only the retained Rust implementation", () => {
  assert.doesNotMatch(source, /full parity is unavailable/);
  assert.doesNotMatch(source, /\bgo test|edge\/go\.mod|cmd\/iotkit-edge/);
  assert.match(source, /cargo test .*iotkit-edge/);
  assert.match(source, /test-edge-console-e2e\.sh/);
  assert.match(source, /test-edge-output\.sh/);
  assert.match(source, /IOTKIT_EDGE_PARITY_REPORT_DIR/);
  assert.match(source, /result\.json/);
});
