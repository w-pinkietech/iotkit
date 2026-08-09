#!/usr/bin/env node

import { pathToFileURL } from "node:url";

const rustFiles = new Set([
  "Cargo.lock",
  "Cargo.toml",
  "rust-toolchain.toml",
]);

// Edge product integration scripts (not trial Docker). Match by prefix so new
// scripts/test-edge-*.sh and scripts/test-rust-edge-*.sh do not fall through to
// allHeavy() and re-introduce trial over-execution (#128).
const edgeScriptPrefixes = [
  "scripts/test-edge-",
  "scripts/test-rust-edge-",
];

// Console frontend / browser journey only (not custody/output integration).
const consoleScriptPrefixes = ["scripts/test-edge-console-"];

// Heavy Edge integration (custody, durable output). Not the Console lane.
const edgeIntegrationScriptPrefixes = ["scripts/test-rust-edge-"];
const edgeIntegrationScriptFiles = new Set(["scripts/test-edge-output.sh"]);
// `main.rs` owns PID1 signal handling. Its changes must run the real SIGTERM
// container gate even though most Edge Node sources only need Rust checks.
const edgeIntegrationSourceFiles = new Set(["edge-node/apps/node/src/main.rs"]);

const trialOnlyFiles = new Set([
  "scripts/iotkit",
  "scripts/iotkit_trial.py",
  "scripts/test-iotkit-trial.sh",
  "deploy/compose.trial.yaml",
  "iotkit.toml",
]);

// Edge paths that implement trial profile behavior (deployment profile, loopback
// guards, CLI). Presentation-only files (templates/CSS) stay on the console lane
// without the trial Docker journey (#166).
const trialRelatedEdgeFiles = new Set([
  "edge/src/cli/mod.rs",
  "edge/src/composition/runtime.rs",
  "edge/src/composition/runtime_config.rs",
  "edge/src/web/mod.rs",
  "edge/tests/cli_contract.rs",
  "edge/tests/console_contract.rs",
  "edge/tests/runtime_composition.rs",
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
  "scripts/check-product-docs.mjs",
  "scripts/product-docs-impact.mjs",
  "scripts/docs/package.json",
  "scripts/docs/package-lock.json",
  "scripts/docs/frontmatter.mjs",
  "scripts/docs/product-docs-impact-rules.json",
  "scripts/tests/product-docs-frontmatter.test.mjs",
  "scripts/tests/product-docs-modes.test.mjs",
  "scripts/tests/product-docs-impact.test.mjs",
  "scripts/check-release-version.mjs",
  "scripts/check-source-layout",
  "scripts/tests/adapter-author-docs.test.mjs",
  "scripts/tests/battle-tested-review.test.mjs",
  "scripts/tests/check-product-docs.test.mjs",
  "scripts/tests/test_iotkit_trial.py",
  "scripts/tests/release-version.test.mjs",
]);

// Longest package roots first. Keep in sync with `cargo metadata --no-deps`.
const packageRoots = [
  ["edge-node/adapters/bravepi-mainboard/codec/", "bravepi-codec"],
  [
    "edge/output-adapters/generic-mqtt-json-v1/",
    "iotkit-output-adapter-generic-mqtt-json-v1",
  ],
  ["edge-node/input/hardware/sensor-drivers/", "iotkit-sensor-drivers"],
  ["edge-node/input/hardware/transports/rpi/", "rpi4b-transport"],
  [
    "edge/output-adapters/pinikiet-mqtt-v1/",
    "iotkit-output-adapter-pinikiet-mqtt-v1",
  ],
  ["edge-node/adapters/bravepi-mainboard/", "bravepi-mainboard-adapter"],
  ["edge-node/input/runtimes/polling/", "iotkit-polling-adapter-runtime"],
  ["edge-node/adapters/trial-sample/", "trial-sample-adapter"],
  ["edge/output-adapters/example/", "iotkit-output-adapter-example"],
  ["edge/output-adapters/testkit/", "iotkit-output-adapter-testkit"],
  ["edge-node/adapters/rpi-local/", "rpi-local-adapter"],
  ["edge-node/tools/bravepi-poc/", "bravepi-poc"],
  ["edge-node/core/supervision/", "iotkit-core-supervision"],
  ["edge-node/core/timeseries/", "iotkit-core-timeseries"],
  ["edge-node/ingest/contract/", "iotkit-ingest-contract"],
  ["edge-node/core/collector/", "iotkit-core-collector"],
  ["edge-node/input/host-api/", "iotkit-input-adapter-host-api"],
  ["edge/output-adapters/api/", "iotkit-output-adapter-api"],
  ["edge-node/core/recovery/", "iotkit-core-recovery"],
  ["edge-node/core/registry/", "iotkit-core-registry"],
  ["edge-node/ingest/client/", "iotkit-ingest-client"],
  ["edge-node/input/testkit/", "iotkit-input-adapter-testkit"],
  ["edge-node/core/publish/", "iotkit-core-publish"],
  ["edge-node/core/storage/", "iotkit-core-storage"],
  ["edge-node/apps/nodectl/", "iotkit-edge-nodectl"],
  ["edge-node/core/engine/", "iotkit-core-engine"],
  ["edge-node/core/ledger/", "iotkit-core-ledger"],
  ["edge/custody-contract/", "iotkit-edge-custody-contract"],
  ["edge-node/ingest/http/", "iotkit-ingest-http"],
  ["edge-node/core/types/", "iotkit-core-types"],
  ["edge-node/apps/node/", "iotkit-edge-node"],
  ["edge-node/core/ops/", "iotkit-core-ops"],
  // iotkit-edge sources only — never catch-all `edge/` (new nested crates under
  // edge/output-adapters/<new>/ must not map to -p iotkit-edge alone).
  ["edge/src/", "iotkit-edge"],
  ["edge/tests/", "iotkit-edge"],
  ["edge/frontend/", "iotkit-edge"],
  ["edge/examples/", "iotkit-edge"],
  ["edge/migrations/", "iotkit-edge"],
  ["edge/openapi/", "iotkit-edge"],
];

// Package manifests / package-root files not covered by directory prefixes above.
const packageFiles = new Map([
  ["edge/Cargo.toml", "iotkit-edge"],
  ["edge/askama.toml", "iotkit-edge"],
]);

// Workspace path-deps that import each package (one hop). From cargo metadata.
const reversePathDeps = {
  "bravepi-codec": ["bravepi-mainboard-adapter"],
  "bravepi-mainboard-adapter": ["bravepi-poc", "iotkit-edge-node"],
  "iotkit-core-collector": [
    "iotkit-core-registry",
    "iotkit-edge-node",
    "iotkit-ingest-client",
    "iotkit-ingest-http",
  ],
  "iotkit-core-engine": ["iotkit-edge-node"],
  "iotkit-core-ledger": [
    "iotkit-core-collector",
    "iotkit-core-ops",
    "iotkit-core-publish",
    "iotkit-core-recovery",
    "iotkit-core-registry",
    "iotkit-core-timeseries",
    "iotkit-edge-node",
    "iotkit-edge-nodectl",
    "iotkit-ingest-client",
    "iotkit-ingest-http",
  ],
  "iotkit-core-ops": [
    "iotkit-core-recovery",
    "iotkit-edge-node",
    "iotkit-edge-nodectl",
    "iotkit-ingest-http",
  ],
  "iotkit-core-publish": [
    "iotkit-core-collector",
    "iotkit-core-ops",
    "iotkit-core-recovery",
    "iotkit-core-registry",
    "iotkit-edge-node",
    "iotkit-edge-nodectl",
    "iotkit-ingest-client",
    "iotkit-ingest-http",
  ],
  "iotkit-core-recovery": ["iotkit-edge-node", "iotkit-edge-nodectl"],
  "iotkit-core-registry": [
    "iotkit-core-ops",
    "iotkit-core-recovery",
    "iotkit-edge-node",
    "iotkit-edge-nodectl",
    "iotkit-ingest-client",
    "iotkit-ingest-http",
    "iotkit-input-adapter-testkit",
  ],
  "iotkit-core-storage": [
    "iotkit-core-collector",
    "iotkit-core-ledger",
    "iotkit-core-ops",
    "iotkit-core-publish",
    "iotkit-core-recovery",
    "iotkit-core-registry",
    "iotkit-core-timeseries",
    "iotkit-edge-node",
    "iotkit-edge-nodectl",
    "iotkit-ingest-client",
    "iotkit-ingest-http",
  ],
  "iotkit-core-supervision": [
    "bravepi-mainboard-adapter",
    "bravepi-poc",
    "iotkit-core-engine",
    "iotkit-edge-node",
  ],
  "iotkit-core-timeseries": [
    "iotkit-core-collector",
    "iotkit-core-ops",
    "iotkit-core-publish",
    "iotkit-core-recovery",
    "iotkit-core-registry",
    "iotkit-edge-node",
    "iotkit-edge-nodectl",
    "iotkit-ingest-client",
    "iotkit-ingest-http",
  ],
  "iotkit-core-types": [
    "bravepi-mainboard-adapter",
    "iotkit-core-engine",
    "iotkit-core-supervision",
    "iotkit-edge-node",
    "iotkit-polling-adapter-runtime",
    "iotkit-sensor-drivers",
    "rpi-local-adapter",
  ],
  "iotkit-edge-custody-contract": ["iotkit-edge"],
  "iotkit-ingest-client": [
    "bravepi-mainboard-adapter",
    "iotkit-edge-node",
    "iotkit-input-adapter-host-api",
    "iotkit-input-adapter-testkit",
  ],
  "iotkit-ingest-contract": [
    "bravepi-mainboard-adapter",
    "iotkit-core-collector",
    "iotkit-core-registry",
    "iotkit-edge-nodectl",
    "iotkit-ingest-client",
    "iotkit-ingest-http",
    "iotkit-input-adapter-host-api",
    "iotkit-input-adapter-testkit",
    "rpi-local-adapter",
    "trial-sample-adapter",
  ],
  "iotkit-ingest-http": ["iotkit-edge-node"],
  "iotkit-input-adapter-host-api": [
    "bravepi-mainboard-adapter",
    "iotkit-edge-node",
    "iotkit-input-adapter-testkit",
    "rpi-local-adapter",
    "trial-sample-adapter",
  ],
  "iotkit-output-adapter-api": [
    "iotkit-edge",
    "iotkit-output-adapter-example",
    "iotkit-output-adapter-generic-mqtt-json-v1",
    "iotkit-output-adapter-pinikiet-mqtt-v1",
    "iotkit-output-adapter-testkit",
  ],
  "iotkit-output-adapter-generic-mqtt-json-v1": ["iotkit-edge"],
  "iotkit-output-adapter-pinikiet-mqtt-v1": ["iotkit-edge"],
  "iotkit-output-adapter-testkit": [
    "iotkit-output-adapter-example",
    "iotkit-output-adapter-generic-mqtt-json-v1",
    "iotkit-output-adapter-pinikiet-mqtt-v1",
  ],
  "iotkit-polling-adapter-runtime": ["rpi-local-adapter"],
  "iotkit-sensor-drivers": ["bravepi-mainboard-adapter", "rpi-local-adapter"],
  "rpi-local-adapter": ["iotkit-edge-node"],
  "rpi4b-transport": ["bravepi-mainboard-adapter", "rpi-local-adapter"],
  "trial-sample-adapter": ["iotkit-edge-node"],
};

// Beyond this many packages after reverse-dep expansion, run the full workspace.
// Shared core crates expand to many consumers; full suite is safer and similar cost.
const MAX_FOCUSED_PACKAGES = 6;

function none() {
  return { rust: false, console: false, edge: false, trial: false };
}

function allHeavy() {
  return { rust: true, console: true, edge: true, trial: true };
}

function isLightweight(path) {
  return (
    lightweightFiles.has(path) ||
    lightweightPrefixes.some((prefix) => path.startsWith(prefix)) ||
    (!path.includes("/") && path.endsWith(".md"))
  );
}

/** Console UI, OpenAPI, and browser-journey surfaces (short CI lane). */
function isConsoleSurfacePath(path) {
  if (consoleScriptPrefixes.some((prefix) => path.startsWith(prefix))) {
    return true;
  }
  if (
    path === "edge/askama.toml" ||
    path === "edge/src/composition/web.rs" ||
    path.startsWith("edge/examples/console")
  ) {
    return true;
  }
  if (path.startsWith("edge/frontend/")) return true;
  if (path.startsWith("edge/src/web/")) return true;
  if (path.startsWith("edge/openapi/")) return true;
  if (
    path === "edge/tests/console_contract.rs" ||
    path === "edge/tests/http_contract.rs" ||
    path === "edge/tests/history_contract.rs"
  ) {
    return true;
  }
  return false;
}

function isEdgeIntegrationScript(path) {
  return (
    edgeIntegrationScriptFiles.has(path) ||
    edgeIntegrationScriptPrefixes.some((prefix) => path.startsWith(prefix))
  );
}

/**
 * Job flags for one changed path.
 * - console: frontend check + browser e2e
 * - edge: custody + durable output integration (not Console)
 * - trial: trial Docker first-run journey
 */
function classify(path) {
  if (
    path === "scripts/select-ci-jobs.mjs" ||
    path === "scripts/tests/select-ci-jobs.test.mjs" ||
    path.startsWith(".github/workflows/") ||
    path === ".config/nextest.toml"
  ) {
    return allHeavy();
  }

  if (isLightweight(path)) {
    return none();
  }

  if (trialOnlyFiles.has(path)) {
    return { rust: false, console: false, edge: false, trial: true };
  }

  if (path.startsWith("edge-node/adapters/trial-sample/")) {
    return { rust: true, console: false, edge: false, trial: true };
  }

  if (edgeIntegrationSourceFiles.has(path)) {
    return { rust: true, console: false, edge: true, trial: false };
  }

  if (path.startsWith("edge-node/")) {
    return { rust: true, console: false, edge: false, trial: false };
  }

  if (path === "edge/Dockerfile") {
    return allHeavy();
  }

  // The Edge manifest owns both browser dependencies (Askama/Axum) and
  // custody/output dependencies (SQLx/MQTT/output adapters).
  if (path === "edge/Cargo.toml") {
    return { rust: true, console: true, edge: true, trial: false };
  }

  // Console lane first so presentation / web UI skips custody+output.
  if (isConsoleSurfacePath(path)) {
    return {
      rust: true,
      console: true,
      edge: false,
      trial: trialRelatedEdgeFiles.has(path),
    };
  }

  if (isEdgeIntegrationScript(path)) {
    return { rust: true, console: false, edge: true, trial: false };
  }

  if (trialRelatedEdgeFiles.has(path)) {
    return { rust: true, console: false, edge: true, trial: true };
  }

  if (edgeScriptPrefixes.some((prefix) => path.startsWith(prefix))) {
    // Other edge product scripts (resilience, bootstrap, …): keep both product
    // lanes; trial stays off unless the path is trial-only above.
    return { rust: true, console: true, edge: true, trial: false };
  }

  if (path.startsWith("edge/")) {
    // Non-console Edge product code: custody/output integration, not browser e2e.
    return { rust: true, console: false, edge: true, trial: false };
  }

  if (rustFiles.has(path)) {
    // Workspace and toolchain changes rebuild trial Docker images as well.
    return allHeavy();
  }

  if (path.startsWith("deploy/") || path === "compose.dev.yaml") {
    return { rust: true, console: true, edge: true, trial: false };
  }

  if (path.startsWith("testdata/")) {
    return allHeavy();
  }

  return allHeavy();
}

function packageForPath(path) {
  if (packageFiles.has(path)) {
    return packageFiles.get(path);
  }

  // Nested Cargo.toml that is not a known package root ⇒ unlisted workspace crate.
  // Focused -p lists would skip it; force full suite via null.
  if (path.endsWith("Cargo.toml")) {
    const listed = packageRoots.find(([root]) => path === `${root}Cargo.toml`);
    if (listed) {
      return listed[1];
    }
    return null;
  }

  for (const [root, name] of packageRoots) {
    if (path === root.slice(0, -1) || path.startsWith(root)) {
      return name;
    }
  }
  return null;
}

function forcesFullRustSuite(path) {
  if (rustFiles.has(path)) return true;
  if (path.startsWith(".github/workflows/")) return true;
  if (path === "scripts/select-ci-jobs.mjs") return true;
  if (path === "scripts/tests/select-ci-jobs.test.mjs") return true;
  if (path === ".config/nextest.toml") return true;
  if (path === "edge/Dockerfile") return true;
  if (path.startsWith("testdata/")) return true;
  if (path.startsWith("deploy/") || path === "compose.dev.yaml") return true;
  if (edgeScriptPrefixes.some((prefix) => path.startsWith(prefix))) return true;
  // Unmapped rust-relevant paths stay on the safe full suite.
  if (classify(path).rust && packageForPath(path) === null) return true;
  return false;
}

function expandPackages(seed) {
  const selected = new Set(seed);
  for (const name of seed) {
    for (const consumer of reversePathDeps[name] ?? []) {
      selected.add(consumer);
    }
  }
  return [...selected].sort();
}

/**
 * Which Cargo packages the rust job should clippy/test.
 * @returns {"all" | string} "all" or comma-separated package names
 */
export function selectRustPackages(paths) {
  const normalized = paths.map((path) => path.trim()).filter(Boolean);
  if (normalized.length === 0) {
    return "all";
  }

  const seeds = new Set();
  for (const path of normalized) {
    if (!classify(path).rust) {
      continue;
    }
    if (forcesFullRustSuite(path)) {
      return "all";
    }
    const pkg = packageForPath(path);
    if (pkg === null) {
      return "all";
    }
    seeds.add(pkg);
  }

  if (seeds.size === 0) {
    return "all";
  }

  const expanded = expandPackages([...seeds]);
  if (expanded.length > MAX_FOCUSED_PACKAGES) {
    return "all";
  }
  return expanded.join(",");
}

export function selectCiJobs(paths) {
  const normalized = paths.map((path) => path.trim()).filter(Boolean);
  if (normalized.length === 0) {
    return { ...allHeavy(), packages: "all" };
  }

  const selected = normalized.reduce(
    (acc, path) => {
      const classification = classify(path);
      return {
        rust: acc.rust || classification.rust,
        console: acc.console || classification.console,
        edge: acc.edge || classification.edge,
        trial: acc.trial || classification.trial,
      };
    },
    none(),
  );

  return {
    ...selected,
    packages: selected.rust ? selectRustPackages(normalized) : "",
  };
}

async function main() {
  let input = "";
  process.stdin.setEncoding("utf8");
  for await (const chunk of process.stdin) {
    input += chunk;
  }
  const selected = selectCiJobs(input.split(/\r?\n/));
  process.stdout.write(
    `rust=${selected.rust}\nconsole=${selected.console}\nedge=${selected.edge}\ntrial=${selected.trial}\npackages=${selected.packages}\n`,
  );
}

if (
  process.argv[1] &&
  pathToFileURL(process.argv[1]).href === import.meta.url
) {
  await main();
}
