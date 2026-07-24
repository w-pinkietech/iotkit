#!/usr/bin/env node

import { pathToFileURL } from "node:url";

const rustRoots = ["edge-node/", "edge/"];

const rustFiles = new Set([
  "Cargo.lock",
  "Cargo.toml",
  "rust-toolchain.toml",
]);

const edgeFiles = new Set([
  "scripts/test-edge-capacity.sh",
  "scripts/test-edge-console-e2e.sh",
  "scripts/test-edge-console-frontend.sh",
  "scripts/test-edge-output.sh",
  "scripts/test-edge-postgres.sh",
  "scripts/test-rust-edge-custody.sh",
  "scripts/test-rust-edge-runtime.sh",
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
  "scripts/check-source-layout",
  "scripts/tests/adapter-author-docs.test.mjs",
  "scripts/tests/battle-tested-review.test.mjs",
]);

function isLightweight(path) {
  return (
    lightweightFiles.has(path) ||
    lightweightPrefixes.some((prefix) => path.startsWith(prefix)) ||
    (!path.includes("/") && path.endsWith(".md"))
  );
}

function classify(path) {
  if (path === "scripts/select-ci-jobs.mjs" ||
      path === "scripts/tests/select-ci-jobs.test.mjs") {
    return { rust: true, edge: true };
  }
  if (isLightweight(path)) {
    return { rust: false, edge: false };
  }
  if (edgeFiles.has(path) || path.startsWith("edge/")) {
    return { rust: true, edge: true };
  }
  if (rustFiles.has(path)) {
    return { rust: true, edge: true };
  }
  if (rustRoots.some((prefix) => path.startsWith(prefix))) {
    return { rust: true, edge: false };
  }
  if (path.startsWith("testdata/") ||
      path.startsWith("deploy/") ||
      path === "compose.dev.yaml") {
    return { rust: true, edge: true };
  }
  return { rust: true, edge: true };
}

export function selectCiJobs(paths) {
  const normalized = paths.map((path) => path.trim()).filter(Boolean);
  if (normalized.length === 0) {
    return { rust: true, edge: true };
  }

  return normalized.reduce(
    (selected, path) => {
      const classification = classify(path);
      return {
        rust: selected.rust || classification.rust,
        edge: selected.edge || classification.edge,
      };
    },
    { rust: false, edge: false },
  );
}

async function main() {
  let input = "";
  process.stdin.setEncoding("utf8");
  for await (const chunk of process.stdin) {
    input += chunk;
  }
  const selected = selectCiJobs(input.split(/\r?\n/));
  process.stdout.write(`rust=${selected.rust}\nedge=${selected.edge}\n`);
}

if (process.argv[1] &&
    pathToFileURL(process.argv[1]).href === import.meta.url) {
  await main();
}
