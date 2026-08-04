import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repositoryRoot = fileURLToPath(new URL("../..", import.meta.url));

test("local Cargo defaults bound compilation and test concurrency", () => {
  const config = readFileSync(
    join(repositoryRoot, ".cargo", "config.toml"),
    "utf8",
  );

  assert.match(config, /^\[build\]\njobs = 4$/m);
  assert.match(
    config,
    /^\[env\]\nRUST_TEST_THREADS = \{ value = "4", force = false \}$/m,
  );
});

test("contributor guides document both explicit overrides", () => {
  for (const guide of ["CONTRIBUTING.md", "CONTRIBUTING.ja.md"]) {
    const contents = readFileSync(join(repositoryRoot, guide), "utf8");
    assert.match(contents, /CARGO_BUILD_JOBS/);
    assert.match(contents, /RUST_TEST_THREADS/);
  }
});

test("lightweight CI protects the local concurrency contract", () => {
  const workflow = readFileSync(
    join(repositoryRoot, ".github", "workflows", "ci.yml"),
    "utf8",
  );
  assert.match(
    workflow,
    /node --test scripts\/tests\/rust-concurrency\.test\.mjs/,
  );
});
