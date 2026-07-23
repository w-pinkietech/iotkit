import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const source = readFileSync(
  new URL("../test-edge-output.sh", import.meta.url),
  "utf8",
);

test("PostgreSQL readiness waits for the final server after init shutdown", () => {
  assert.match(
    source,
    /PostgreSQL init process complete; ready for start up\./,
  );
  assert.match(source, /pg_isready/);
});
