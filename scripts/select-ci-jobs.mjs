#!/usr/bin/env node

import { pathToFileURL } from "node:url";

const rustFiles = new Set([
  "Cargo.lock",
  "Cargo.toml",
  "rust-toolchain.toml",
]);

const edgeScripts = new Set([
  "scripts/test-edge-capacity.sh",
  "scripts/test-edge-console-e2e.sh",
  "scripts/test-edge-console-frontend.sh",
  "scripts/test-edge-output.sh",
  "scripts/test-edge-postgres.sh",
  "scripts/test-rust-edge-custody.sh",
  "scripts/test-rust-edge-runtime.sh",
]);

const trialOnlyFiles = new Set([
  "scripts/iotkit",
  "scripts/iotkit_trial.py",
  "scripts/test-iotkit-trial.sh",
  "deploy/compose.trial.yaml",
  "iotkit.toml",
]);

const lightweightPrefixes = [
  ".agents/",
  ".codex/",
  ".github/ISSUE_TEMPLATE/",
  "docs/",
  "review/",
];

const lightweightFiles = new Set([
  ".gitignore",
  "LICENSE",
  "scripts/battle-tested-review.mjs",
  "scripts/check-layers",
  "scripts/check-okf-docs.mjs",
  "scripts/check-release-version.mjs",
  "scripts/check-source-layout",
  "scripts/tests/adapter-author-docs.test.mjs",
  "scripts/tests/battle-tested-review.test.mjs",
  "scripts/tests/test_iotkit_trial.py",
  "scripts/tests/release-version.test.mjs",
]);

function none() {
  return { rust: false, edge: false, trial: false };
}

function allHeavy() {
  return { rust: true, edge: true, trial: true };
}

function isLightweight(path) {
  return (
    lightweightFiles.has(path) ||
    lightweightPrefixes.some((prefix) => path.startsWith(prefix)) ||
    (!path.includes("/") && path.endsWith(".md"))
  );
}

function classify(path) {
  if (
    path === "scripts/select-ci-jobs.mjs" ||
    path === "scripts/tests/select-ci-jobs.test.mjs" ||
    path.startsWith(".github/workflows/")
  ) {
    return allHeavy();
  }

  if (isLightweight(path)) {
    return none();
  }

  if (trialOnlyFiles.has(path)) {
    return { rust: false, edge: false, trial: true };
  }

  if (path.startsWith("edge-node/adapters/trial-sample/")) {
    return { rust: true, edge: false, trial: true };
  }

  if (path.startsWith("edge-node/")) {
    return { rust: true, edge: false, trial: false };
  }

  if (path === "edge/Dockerfile") {
    return allHeavy();
  }

  if (path.startsWith("edge/") || edgeScripts.has(path)) {
    return { rust: true, edge: true, trial: false };
  }

  if (rustFiles.has(path)) {
    // Workspace and toolchain changes rebuild trial Docker images as well.
    return allHeavy();
  }

  if (path.startsWith("deploy/") || path === "compose.dev.yaml") {
    return { rust: true, edge: true, trial: false };
  }

  if (path.startsWith("testdata/")) {
    return allHeavy();
  }

  return allHeavy();
}

export function selectCiJobs(paths) {
  const normalized = paths.map((path) => path.trim()).filter(Boolean);
  if (normalized.length === 0) {
    return allHeavy();
  }

  return normalized.reduce(
    (selected, path) => {
      const classification = classify(path);
      return {
        rust: selected.rust || classification.rust,
        edge: selected.edge || classification.edge,
        trial: selected.trial || classification.trial,
      };
    },
    none(),
  );
}

async function main() {
  let input = "";
  process.stdin.setEncoding("utf8");
  for await (const chunk of process.stdin) {
    input += chunk;
  }
  const selected = selectCiJobs(input.split(/\r?\n/));
  process.stdout.write(
    `rust=${selected.rust}\nedge=${selected.edge}\ntrial=${selected.trial}\n`,
  );
}

if (
  process.argv[1] &&
  pathToFileURL(process.argv[1]).href === import.meta.url
) {
  await main();
}
