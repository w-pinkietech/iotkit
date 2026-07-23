import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import test from "node:test";

const metadata = JSON.parse(
  execFileSync(
    "cargo",
    ["metadata", "--no-deps", "--format-version", "1"],
    { encoding: "utf8" },
  ),
);

test("the workspace exposes a testable Rust IoTKit Edge application", () => {
  const edge = metadata.packages.find(({ name }) => name === "iotkit-edge");
  assert.ok(edge, "missing iotkit-edge Rust package");
  assert.ok(edge.targets.some(({ kind }) => kind.includes("lib")));
  assert.ok(
    edge.targets.some(
      ({ kind, name }) => kind.includes("bin") && name === "iotkit-edge",
    ),
  );
});
