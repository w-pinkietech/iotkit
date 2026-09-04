#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { lstatSync, readFileSync, readdirSync, realpathSync } from "node:fs";
import { dirname, isAbsolute, relative, resolve, sep } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const EXPECTED_REPOSITORY = "https://github.com/w-pinkietech/iotkit";
const COMPATIBILITY_MANIFEST_PATH = "testdata/compatibility/v1/release-manifest.json";

export const REQUIRED_COMPATIBILITY_DOMAINS = [
  "http-ingest-v1",
  "input-adapter-v1",
  "mqtt-output-adapter-v1",
];

export const REQUIRED_STORAGE_SCHEMAS = ["edge-node-sqlite"];

const MANIFEST_KEYS = ["schema_version", "domains", "storage"];
const DOMAIN_KEYS = ["id", "authority", "types", "schemas", "fixtures", "tests"];
const STORAGE_KEYS = ["id", "schema_version", "authority", "schema", "tests"];
const DOMAIN_REQUIRED_EVIDENCE = ["authority", "types", "schemas", "tests"];
const STORAGE_REQUIRED_EVIDENCE = ["authority", "schema", "tests"];
const ENGLISH_README_RELEASE_PATTERN =
  /^> \*\*Current product version: ([^ ]+) \((pre-1\.0|stable)\)\.\*\*(.*(?:\n>.*)*)/m;
const JAPANESE_README_RELEASE_PATTERN =
  /^> \*\*現在の製品バージョン: ([^（]+)（(pre-1\.0|stable)）。\*\*(.*(?:\n>.*)*)/m;

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

function extractReadmeRelease(readme, pattern, error) {
  const match = readme.match(pattern);
  if (!match) throw new Error(error);
  return { version: match[1], marker: match[2], status: match[0] };
}

function parseEnglishReadmeRelease(readme) {
  return extractReadmeRelease(
    readme,
    ENGLISH_README_RELEASE_PATTERN,
    "README.md is missing the current product version marker",
  );
}

export function extractEnglishReadmeRelease(readme) {
  const { version, marker } = parseEnglishReadmeRelease(readme);
  return { version, marker };
}

export function extractEnglishReadmeVersion(readme) {
  return extractEnglishReadmeRelease(readme).version;
}

function parseJapaneseReadmeRelease(readme) {
  return extractReadmeRelease(
    readme,
    JAPANESE_README_RELEASE_PATTERN,
    "README.ja.md is missing the current product version marker",
  );
}

export function extractJapaneseReadmeRelease(readme) {
  const { version, marker } = parseJapaneseReadmeRelease(readme);
  return { version, marker };
}

export function extractJapaneseReadmeVersion(readme) {
  return extractJapaneseReadmeRelease(readme).version;
}

function requiredReadmeMarker(version) {
  const major = /^(0|[1-9]\d*)\./.exec(version)?.[1];
  if (major === undefined) return undefined;
  return major === "0" ? "pre-1.0" : "stable";
}

function validateReadmeStatusBlock({
  status,
  label,
  version,
  marker,
  stablePhrase,
  pre1Phrase,
  problems,
}) {
  if (status === undefined) return;
  if (typeof status !== "string") {
    problems.push(`${label} status block must be a string`);
    return;
  }
  if (marker === "stable" && !status.includes(stablePhrase)) {
    problems.push(`${label} status block must describe a stable source release for ${version}`);
  }
  if (marker === "stable" && status.includes(pre1Phrase)) {
    problems.push(`${label} status block still contains pre-1.0 wording for ${version}`);
  }
  if (marker === "pre-1.0" && !status.includes(pre1Phrase)) {
    problems.push(`${label} status block must describe an early source release for ${version}`);
  }
}

export function extractChangelogReleases(changelog) {
  return [
    ...changelog.matchAll(/^## \[([^\]]+)\](?: - (.*))?$/gm),
  ]
    .filter((match) => match[1] !== "Unreleased")
    .map((match) => ({
      version: match[1],
      date: match[2] ?? "",
    }));
}

export function extractChangelogVersions(changelog) {
  return extractChangelogReleases(changelog).map((release) => release.version);
}

export function packageInheritsWorkspaceVersion(cargoToml) {
  const packageSection = cargoToml.match(
    /^\[package\]\r?\n((?:(?!^\[)[\s\S])*)/m,
  )?.[1];
  return /^version\.workspace\s*=\s*true\s*$/m.test(packageSection ?? "");
}

function isObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function validateExactKeys(value, allowed, label, problems) {
  if (!isObject(value)) {
    problems.push(`${label} must be an object`);
    return false;
  }
  for (const key of Object.keys(value)) {
    if (!allowed.includes(key)) problems.push(`${label} has unexpected key ${key}`);
  }
  for (const key of allowed) {
    if (!(key in value)) problems.push(`${label} is missing required key ${key}`);
  }
  return true;
}

function isSafeRepositoryRelativePath(value) {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value === value.trim() &&
    !value.startsWith("/") &&
    !value.includes("\\") &&
    !value.split("/").some((part) => part === "" || part === "." || part === "..")
  );
}

function isWithinRoot(root, target) {
  const pathFromRoot = relative(root, target);
  return (
    pathFromRoot !== "" &&
    pathFromRoot !== ".." &&
    !pathFromRoot.startsWith(`..${sep}`) &&
    !isAbsolute(pathFromRoot)
  );
}

function validateEvidencePaths(value, label, root, problems, required) {
  if (!Array.isArray(value)) {
    problems.push(`${label} must be an array`);
    return;
  }
  if (required && value.length === 0) {
    problems.push(`${label} must not be empty`);
  }
  const realRoot = realpathSync(root);
  for (const path of value) {
    if (!isSafeRepositoryRelativePath(path)) {
      problems.push(`${label} must be a safe repository-relative path: ${String(path)}`);
      continue;
    }
    const target = resolve(root, path);
    if (!isWithinRoot(root, target)) {
      problems.push(`${label} must be a safe repository-relative path: ${path}`);
      continue;
    }
    try {
      if (lstatSync(target).isSymbolicLink()) {
        problems.push(`${label} must not reference a symbolic link: ${path}`);
      } else if (!isWithinRoot(realRoot, realpathSync(target))) {
        problems.push(`${label} resolves outside the repository root: ${path}`);
      }
    } catch {
      problems.push(`${label} does not exist: ${path}`);
    }
  }
}

function validateEvidenceEntry(entry, keys, requiredEvidence, label, root, problems) {
  if (!validateExactKeys(entry, keys, label, problems)) return;
  if (typeof entry.id !== "string" || !entry.id) {
    problems.push(`${label}.id must be a non-empty string`);
  }
  for (const key of keys.filter((key) => !["id", "schema_version"].includes(key))) {
    validateEvidencePaths(
      entry[key],
      `${label}.${key}`,
      root,
      problems,
      requiredEvidence.includes(key),
    );
  }
}

function validateStorageMigrationVersion(storage, label, root, problems) {
  if (
    !Number.isSafeInteger(storage?.schema_version) ||
    storage.schema_version < 1 ||
    !Array.isArray(storage?.schema) ||
    storage.schema.length === 0
  ) {
    return;
  }

  const realRoot = realpathSync(root);
  let latestVersion;
  for (const path of storage.schema) {
    if (!isSafeRepositoryRelativePath(path)) continue;
    const directory = resolve(root, path);
    if (!isWithinRoot(root, directory)) continue;
    try {
      const metadata = lstatSync(directory);
      if (!metadata.isDirectory()) {
        if (!metadata.isSymbolicLink()) {
          problems.push(`${label}.schema must reference migration directories: ${path}`);
        }
        continue;
      }
      if (!isWithinRoot(realRoot, realpathSync(directory))) continue;
      for (const entry of readdirSync(directory, { withFileTypes: true })) {
        const match = entry.name.match(/^(\d+)_.*\.sql$/);
        if (!match) continue;
        if (entry.isSymbolicLink()) {
          problems.push(`${label}.schema must not contain symbolic-link migration ${entry.name}`);
          continue;
        }
        if (!entry.isFile()) continue;
        const version = Number(match[1]);
        if (!Number.isSafeInteger(version) || version < 1) {
          problems.push(`${label}.schema has invalid migration filename ${path}/${entry.name}`);
          continue;
        }
        latestVersion = Math.max(latestVersion ?? 0, version);
      }
    } catch {
      // The evidence-path validation above reports the missing or unreadable path.
    }
  }
  if (latestVersion === undefined) {
    problems.push(`${label}.schema does not contain a numeric migration filename`);
  } else if (storage.schema_version !== latestVersion) {
    problems.push(
      `${label}.schema_version ${storage.schema_version} does not match migration version ${latestVersion}`,
    );
  }
}

/// Validates that one source release contains a closed compatibility evidence index.
///
/// The index deliberately records contract and storage versions separately from the
/// product version. Release tags bind the product version to this evidence.
export function validateCompatibilityManifest(manifest, root) {
  const problems = [];
  if (!validateExactKeys(manifest, MANIFEST_KEYS, "compatibility manifest", problems)) {
    return problems;
  }
  if (manifest.schema_version !== 1) {
    problems.push("compatibility manifest schema_version must be 1");
  }

  const domainIds = new Set();
  if (!Array.isArray(manifest.domains)) {
    problems.push("compatibility manifest.domains must be an array");
  } else {
    for (const [index, domain] of manifest.domains.entries()) {
      const label = `compatibility manifest.domains[${index}]`;
      validateEvidenceEntry(
        domain,
        DOMAIN_KEYS,
        DOMAIN_REQUIRED_EVIDENCE,
        label,
        root,
        problems,
      );
      if (typeof domain?.id === "string") {
        if (domainIds.has(domain.id)) problems.push(`${label} has duplicate id ${domain.id}`);
        domainIds.add(domain.id);
      }
    }
  }
  for (const id of REQUIRED_COMPATIBILITY_DOMAINS) {
    if (!domainIds.has(id)) problems.push(`compatibility manifest is missing required domain ${id}`);
  }

  const storageIds = new Set();
  if (!Array.isArray(manifest.storage)) {
    problems.push("compatibility manifest.storage must be an array");
  } else {
    for (const [index, storage] of manifest.storage.entries()) {
      const label = `compatibility manifest.storage[${index}]`;
      validateEvidenceEntry(
        storage,
        STORAGE_KEYS,
        STORAGE_REQUIRED_EVIDENCE,
        label,
        root,
        problems,
      );
      if (!Number.isSafeInteger(storage?.schema_version) || storage.schema_version < 1) {
        problems.push(`${label}.schema_version must be a positive safe integer`);
      }
      validateStorageMigrationVersion(storage, label, root, problems);
      if (typeof storage?.id === "string") {
        if (storageIds.has(storage.id)) problems.push(`${label} has duplicate id ${storage.id}`);
        storageIds.add(storage.id);
      }
    }
  }
  for (const id of REQUIRED_STORAGE_SCHEMAS) {
    if (!storageIds.has(id)) {
      problems.push(`compatibility manifest is missing required storage schema ${id}`);
    }
  }
  return problems;
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
  const expectedReadmeMarker = requiredReadmeMarker(state.version);
  if (expectedReadmeMarker && state.readmeMarker !== undefined && state.readmeMarker !== expectedReadmeMarker) {
    problems.push(
      `README product lifecycle marker is ${state.readmeMarker}, expected ${expectedReadmeMarker} for ${state.version}`,
    );
  }
  if (expectedReadmeMarker && state.readmeJaMarker !== undefined && state.readmeJaMarker !== expectedReadmeMarker) {
    problems.push(
      `README.ja product lifecycle marker is ${state.readmeJaMarker}, expected ${expectedReadmeMarker} for ${state.version}`,
    );
  }
  validateReadmeStatusBlock({
    status: state.readmeStatus,
    label: "README",
    version: state.version,
    marker: expectedReadmeMarker,
    stablePhrase: "stable source release",
    pre1Phrase: "early source release",
    problems,
  });
  validateReadmeStatusBlock({
    status: state.readmeJaStatus,
    label: "README.ja",
    version: state.version,
    marker: expectedReadmeMarker,
    stablePhrase: "安定source release",
    pre1Phrase: "早期source release",
    problems,
  });
  if (
    state.readmeVersion !== undefined &&
    state.readmeVersion !== state.version
  ) {
    problems.push(
      `README product version is ${state.readmeVersion}, expected ${state.version}`,
    );
  }
  if (
    state.readmeJaVersion !== undefined &&
    state.readmeJaVersion !== state.version
  ) {
    problems.push(
      `README.ja product version is ${state.readmeJaVersion}, expected ${state.version}`,
    );
  }
  const changelogReleases =
    state.changelogReleases ??
    state.changelogVersions?.map((version) => ({ version }));
  if (
    changelogReleases !== undefined &&
    !changelogReleases.some((release) => release.version === state.version)
  ) {
    problems.push(
      `CHANGELOG.md has no release heading for ${state.version}`,
    );
  }
  const seenChangelogVersions = new Set();
  for (const release of changelogReleases ?? []) {
    if (seenChangelogVersions.has(release.version)) {
      problems.push(
        `CHANGELOG.md has duplicate release heading for ${release.version}`,
      );
    }
    seenChangelogVersions.add(release.version);
  }
  for (const release of changelogReleases ?? []) {
    if (
      !/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(release.version)
    ) {
      problems.push(`CHANGELOG.md version is not SemVer: ${release.version}`);
    }
    if (
      release.date !== undefined &&
      !/^\d{4}-\d{2}-\d{2}$/.test(release.date)
    ) {
      problems.push(
        `CHANGELOG.md release date is invalid for ${release.version}: ${release.date || "(missing)"}`,
      );
    }
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
  const readme = readFileSync(resolve(root, "README.md"), "utf8");
  const readmeJa = readFileSync(resolve(root, "README.ja.md"), "utf8");
  const changelog = readFileSync(resolve(root, "CHANGELOG.md"), "utf8");
  const readmeRelease = parseEnglishReadmeRelease(readme);
  const readmeJaRelease = parseJapaneseReadmeRelease(readmeJa);
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
        inheritsVersion: packageInheritsWorkspaceVersion(manifest),
      };
    });

  return {
    version: extractWorkspaceVersion(rootManifest),
    packages,
    repository: extractWorkspaceField(rootManifest, "repository"),
    tag,
    readmeVersion: readmeRelease.version,
    readmeMarker: readmeRelease.marker,
    readmeStatus: readmeRelease.status,
    readmeJaVersion: readmeJaRelease.version,
    readmeJaMarker: readmeJaRelease.marker,
    readmeJaStatus: readmeJaRelease.status,
    changelogReleases: extractChangelogReleases(changelog),
  };
}

function loadCompatibilityManifest(root) {
  return JSON.parse(readFileSync(resolve(root, COMPATIBILITY_MANIFEST_PATH), "utf8"));
}

function main() {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const tag = parseTag(process.argv.slice(2));
  const problems = [
    ...validateReleaseState(loadReleaseState(root, tag)),
    ...validateCompatibilityManifest(loadCompatibilityManifest(root), root),
  ];
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
