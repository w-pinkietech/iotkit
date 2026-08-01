import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const checker = path.join(repoRoot, "scripts/check-product-docs.mjs");

function run(mode) {
  return spawnSync(process.execPath, [checker, `--mode=${mode}`], {
    cwd: repoRoot,
    encoding: "utf8",
  });
}

test("default all mode and iotkit-product pass on the repository corpus", () => {
  for (const mode of ["all", "iotkit-product", "okf-min"]) {
    const result = run(mode);
    assert.equal(result.status, 0, result.stderr || result.stdout);
    assert.match(result.stdout, new RegExp(`\\[mode=${mode}\\]`));
  }
});

test("unknown mode exits 2", () => {
  const result = spawnSync(process.execPath, [checker, "--mode=nope"], {
    cwd: repoRoot,
    encoding: "utf8",
  });
  assert.equal(result.status, 2);
});
