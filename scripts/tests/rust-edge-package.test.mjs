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

test("the IoTKit Edge product no longer carries a Go implementation", () => {
  const tracked = execFileSync(
    "git",
    ["ls-files", "edge"],
    { encoding: "utf8" },
  )
    .trim()
    .split("\n")
    .filter(Boolean);

  assert.equal(tracked.some((path) => path.endsWith(".go")), false);
  assert.equal(tracked.includes("edge/go.mod"), false);
  assert.equal(tracked.includes("edge/go.sum"), false);
});
