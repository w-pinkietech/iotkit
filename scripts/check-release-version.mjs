#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const EXPECTED_REPOSITORY = "https://github.com/w-pinkietech/iotkit";

function extractWorkspaceField(cargoToml, field) {
  const section = cargoToml.match(
    /^\[workspace\.package\]\r?\n((?:(?!^\[)[\s\S])*)/m,
  );
  return section?.[1].match(
    new RegExp(`^${field}\\s*=\\s*"([^"]+)"\\s*$`, "m"),
  )?.[1];
}

export function extractWorkspaceVersion(cargoToml) {
  const version = extractWorkspaceField(cargoToml, "version");
  if (!version) {
    throw new Error(
      "Cargo.toml is missing [workspace.package] version",
    );
  }
  return version;
}

export function validateReleaseState(state) {
  const problems = [];
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(state.version)) {
    problems.push(
      `workspace version is not MAJOR.MINOR.PATCH SemVer: ${state.version}`,
    );
  }
  for (const pkg of state.packages) {
    if (pkg.version !== state.version) {
      problems.push(
        `${pkg.name} resolves to ${pkg.version}, expected ${state.version}`,
      );
    }
    if (!pkg.inheritsVersion) {
      problems.push(`${pkg.name} does not use version.workspace = true`);
    }
  }
  if (state.repository !== EXPECTED_REPOSITORY) {
    problems.push(
      `workspace repository URL must be ${EXPECTED_REPOSITORY}`,
    );
  }
  if (state.tag && state.tag !== `v${state.version}`) {
    problems.push(`tag must be v${state.version}`);
  }
  return problems;
}

function parseTag(args) {
  if (args.length === 0) return undefined;
  if (args.length === 2 && args[0] === "--tag" && args[1]) return args[1];
  throw new Error("usage: node scripts/check-release-version.mjs [--tag vX.Y.Z]");
}

function loadReleaseState(root, tag) {
  const rootManifest = readFileSync(resolve(root, "Cargo.toml"), "utf8");
  const metadata = JSON.parse(
    execFileSync(
      "cargo",
      ["metadata", "--locked", "--no-deps", "--format-version", "1"],
      { cwd: root, encoding: "utf8" },
    ),
  );
  const workspaceMembers = new Set(metadata.workspace_members);
  const packages = metadata.packages
    .filter((pkg) => workspaceMembers.has(pkg.id))
    .map((pkg) => {
      const manifest = readFileSync(pkg.manifest_path, "utf8");
      return {
        name: pkg.name,
        version: pkg.version,
        inheritsVersion: /^version\.workspace\s*=\s*true\s*$/m.test(manifest),
      };
    });

  return {
    version: extractWorkspaceVersion(rootManifest),
    packages,
    repository: extractWorkspaceField(rootManifest, "repository"),
    tag,
  };
}

function main() {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const tag = parseTag(process.argv.slice(2));
  const problems = validateReleaseState(loadReleaseState(root, tag));
  if (problems.length > 0) {
    for (const problem of problems) console.error(`release version: ${problem}`);
    process.exitCode = 1;
    return;
  }
  console.log(
    `release version ${extractWorkspaceVersion(readFileSync(resolve(root, "Cargo.toml"), "utf8"))} is consistent`,
  );
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(resolve(process.argv[1])).href
) {
  try {
    main();
  } catch (error) {
    console.error(`release version: ${error.message}`);
    process.exitCode = 1;
  }
}
