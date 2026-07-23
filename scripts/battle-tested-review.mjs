#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, isAbsolute, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptPath = fileURLToPath(import.meta.url);
const defaultRepoRoot = resolve(dirname(scriptPath), "..");
const catalogRelativePath = "review/battle-tested/catalog.json";
const evidenceLevels = new Set([
  "hypothesis",
  "field-reported",
  "field-observed",
  "reproduced",
]);
const forbiddenPrefixes = new Set(["", ".", "./", "*", "**", "**/*"]);

function normalizedPath(value) {
  return value.replaceAll("\\", "/").replace(/^\.\/+/, "");
}

export function loadCatalog(repoRoot = defaultRepoRoot) {
  return JSON.parse(
    readFileSync(resolve(repoRoot, catalogRelativePath), "utf8"),
  );
}

function nonEmptyStrings(value) {
  return (
    Array.isArray(value) &&
    value.length > 0 &&
    value.every((item) => typeof item === "string" && item.trim() !== "")
  );
}

function referenceExists(reference, repoRoot) {
  if (/^https:\/\/github\.com\/[^/]+\/[^/]+\/issues\/[1-9][0-9]*$/.test(reference)) {
    return true;
  }
  if (/^[a-z]+:\/\//i.test(reference) || isAbsolute(reference)) {
    return false;
  }
  return existsSync(resolve(repoRoot, reference));
}

export function validateCatalog(catalog, repoRoot = defaultRepoRoot) {
  const errors = [];
  if (catalog?.schema_version !== 1) {
    errors.push("schema_version must be 1");
  }
  if (!Array.isArray(catalog?.entries) || catalog.entries.length === 0) {
    errors.push("entries must be a non-empty array");
    return errors;
  }
  if (!nonEmptyStrings(catalog?.concern_vocabulary)) {
    errors.push("concern_vocabulary must be non-empty strings");
  }
  const concernVocabulary = new Set(catalog?.concern_vocabulary ?? []);
  if (concernVocabulary.size !== (catalog?.concern_vocabulary ?? []).length) {
    errors.push("concern_vocabulary must not contain duplicates");
  }

  const ids = new Set();
  for (const [index, entry] of catalog.entries.entries()) {
    const location = `entries[${index}]`;
    if (!/^BT-[0-9]{3}$/.test(entry?.id ?? "")) {
      errors.push(`${location}.id must match BT-NNN`);
    } else if (ids.has(entry.id)) {
      errors.push(`${location}.id duplicates ${entry.id}`);
    } else {
      ids.add(entry.id);
    }

    if (typeof entry?.title !== "string" || entry.title.trim() === "") {
      errors.push(`${location}.title must be a non-empty string`);
    }
    if (!evidenceLevels.has(entry?.evidence_level)) {
      errors.push(`${location}.evidence_level is invalid`);
    }
    if (!nonEmptyStrings(entry?.concerns)) {
      errors.push(`${location}.concerns must be non-empty strings`);
    } else {
      for (const concern of entry.concerns) {
        if (!concernVocabulary.has(concern)) {
          errors.push(`${location}.concerns is not in concern_vocabulary: ${concern}`);
        }
      }
    }
    if (!nonEmptyStrings(entry?.path_prefixes)) {
      errors.push(`${location}.path_prefixes must be non-empty strings`);
    } else {
      for (const prefix of entry.path_prefixes) {
        const normalized = normalizedPath(prefix);
        if (forbiddenPrefixes.has(normalized)) {
          errors.push(`${location}.path_prefixes contains catch-all ${prefix}`);
          continue;
        }
        const target = normalized.endsWith("/")
          ? normalized.slice(0, -1)
          : normalized;
        if (!existsSync(resolve(repoRoot, target))) {
          errors.push(`${location}.path_prefixes target does not exist: ${prefix}`);
        }
      }
    }
    if (
      typeof entry?.review_question !== "string" ||
      entry.review_question.trim() === ""
    ) {
      errors.push(`${location}.review_question must be a non-empty string`);
    }
    if (
      typeof entry?.coverage_gap !== "string" ||
      entry.coverage_gap.trim() === ""
    ) {
      errors.push(`${location}.coverage_gap must be a non-empty string`);
    }
    if (!nonEmptyStrings(entry?.provenance)) {
      errors.push(`${location}.provenance must be non-empty strings`);
    } else {
      for (const reference of entry.provenance) {
        if (!referenceExists(reference, repoRoot)) {
          errors.push(`${location}.provenance does not exist: ${reference}`);
        }
      }
    }
    if (!Array.isArray(entry?.guards)) {
      errors.push(`${location}.guards must be an array`);
    } else {
      for (const reference of entry.guards) {
        if (typeof reference !== "string" || !referenceExists(reference, repoRoot)) {
          errors.push(`${location}.guards does not exist: ${reference}`);
        }
      }
    }
  }
  return errors;
}

function pathMatches(path, prefix) {
  const normalized = normalizedPath(prefix);
  return normalized.endsWith("/")
    ? path.startsWith(normalized)
    : path === normalized;
}

export function selectEntries(catalog, paths = [], concerns = []) {
  const normalizedPaths = [...new Set(paths.map(normalizedPath).filter(Boolean))];
  const requestedConcerns = [...new Set(concerns.filter(Boolean))];
  const selections = [];

  for (const entry of catalog.entries) {
    const matchedPaths = normalizedPaths.filter((path) =>
      entry.path_prefixes.some((prefix) => pathMatches(path, prefix)),
    );
    const matchedConcerns = requestedConcerns.filter((concern) =>
      entry.concerns.includes(concern),
    );
    if (matchedPaths.length > 0 || matchedConcerns.length > 0) {
      selections.push({ entry, matchedPaths, matchedConcerns });
    }
  }

  const unmatchedPaths = normalizedPaths.filter(
    (path) =>
      !catalog.entries.some((entry) =>
        entry.path_prefixes.some((prefix) => pathMatches(path, prefix)),
      ),
  );
  const knownConcerns = new Set(catalog.concern_vocabulary);
  const unknownConcerns = requestedConcerns.filter(
    (concern) => !knownConcerns.has(concern),
  );
  const concernsWithoutEntries = requestedConcerns.filter(
    (concern) =>
      knownConcerns.has(concern) &&
      !catalog.entries.some((entry) => entry.concerns.includes(concern)),
  );
  return { selections, unmatchedPaths, unknownConcerns, concernsWithoutEntries };
}

export function parseNameStatus(output) {
  const paths = [];
  for (const line of output.split(/\r?\n/).filter(Boolean)) {
    const [status, firstPath, secondPath] = line.split("\t");
    if (!status || !firstPath) {
      continue;
    }
    paths.push(firstPath);
    if ((status.startsWith("R") || status.startsWith("C")) && secondPath) {
      paths.push(secondPath);
    }
  }
  return paths;
}

function gitNameStatusPaths(repoRoot, args) {
  const output = execFileSync("git", args, {
    cwd: repoRoot,
    encoding: "utf8",
  });
  return parseNameStatus(output);
}

function gitLines(repoRoot, args) {
  const output = execFileSync("git", args, {
    cwd: repoRoot,
    encoding: "utf8",
  });
  return output.split(/\r?\n/).filter(Boolean);
}

function changedPaths(repoRoot, base) {
  return [
    ...new Set([
      ...gitNameStatusPaths(repoRoot, [
        "diff",
        "--name-status",
        "--find-renames",
        "--diff-filter=ACMRD",
        `${base}...HEAD`,
      ]),
      ...gitNameStatusPaths(repoRoot, [
        "diff",
        "--name-status",
        "--find-renames",
        "--diff-filter=ACMRD",
      ]),
      ...gitNameStatusPaths(repoRoot, [
        "diff",
        "--cached",
        "--name-status",
        "--find-renames",
        "--diff-filter=ACMRD",
      ]),
      ...gitLines(repoRoot, [
        "ls-files",
        "--others",
        "--exclude-standard",
      ]),
    ]),
  ];
}

function parseSelectArgs(args) {
  let base = "";
  const concerns = [];
  const paths = [];
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index];
    const value = args[index + 1];
    if (argument === "--base" && value) {
      base = value;
      index += 1;
    } else if (argument === "--concern" && value) {
      concerns.push(value);
      index += 1;
    } else if (argument === "--paths" && value) {
      paths.push(...value.split(",").filter(Boolean));
      index += 1;
    } else {
      throw new Error(`unknown or incomplete argument: ${argument}`);
    }
  }
  if (!base && paths.length === 0) {
    throw new Error("select requires --base REF or --paths PATH[,PATH]");
  }
  return { base, concerns, paths };
}

function printSelection(result, paths) {
  const limited = (values, limit = 5) => {
    const visible = values.slice(0, limit);
    const remainder = values.length - visible.length;
    return remainder > 0
      ? `${visible.join(", ")}, and ${remainder} more`
      : visible.join(", ");
  };
  console.log("Battle-tested review selection");
  console.log(`Changed paths: ${paths.length}`);
  if (result.selections.length === 0) {
    console.log("Selected entries: none");
  }
  for (const { entry, matchedPaths, matchedConcerns } of result.selections) {
    const reasons = [
      ...matchedPaths.map((path) => `path:${path}`),
      ...matchedConcerns.map((concern) => `concern:${concern}`),
    ];
    console.log(`\n${entry.id} [${entry.evidence_level}] ${entry.title}`);
    console.log(`Selected by: ${limited(reasons)}`);
    console.log(`Review: ${entry.review_question}`);
    console.log(`Coverage gap: ${entry.coverage_gap}`);
    if (entry.guards.length > 0) {
      console.log(`Existing guards: ${entry.guards.join(", ")}`);
    } else {
      console.log("Existing guards: none");
    }
  }
  if (result.unmatchedPaths.length > 0) {
    console.log("\nUnmatched paths:");
    for (const path of result.unmatchedPaths.slice(0, 20)) {
      console.log(`- ${path}`);
    }
    if (result.unmatchedPaths.length > 20) {
      console.log(`- and ${result.unmatchedPaths.length - 20} more`);
    }
  }
  if (result.unknownConcerns.length > 0) {
    console.log("\nUnknown concerns:");
    for (const concern of result.unknownConcerns) {
      console.log(`- ${concern}`);
    }
  }
  if (result.concernsWithoutEntries.length > 0) {
    console.log("\nRecognized concerns without a current review entry:");
    for (const concern of result.concernsWithoutEntries) {
      console.log(`- ${concern}`);
    }
  }
  console.log(
    "\nPath routing is a lower bound. No match does not mean the change is safe.",
  );
}

function main(args) {
  const command = args[0];
  const catalog = loadCatalog();
  const errors = validateCatalog(catalog);
  if (errors.length > 0) {
    for (const error of errors) {
      console.error(`review catalog: ${error}`);
    }
    process.exitCode = 1;
    return;
  }

  if (command === "check") {
    console.log(
      `battle-tested review: OK (${catalog.entries.length} entries, all references exist)`,
    );
    return;
  }
  if (command === "concerns") {
    const concerns = [...catalog.concern_vocabulary].sort();
    for (const concern of concerns) {
      console.log(concern);
    }
    return;
  }
  if (command === "select") {
    const selectionArgs = parseSelectArgs(args.slice(1));
    const paths = selectionArgs.base
      ? [...new Set([...changedPaths(defaultRepoRoot, selectionArgs.base), ...selectionArgs.paths])]
      : selectionArgs.paths;
    const result = selectEntries(catalog, paths, selectionArgs.concerns);
    printSelection(result, paths);
    if (result.unknownConcerns.length > 0) {
      process.exitCode = 2;
    }
    return;
  }
  console.error(
    "usage: node scripts/battle-tested-review.mjs check | concerns | select --base REF [--concern NAME] [--paths PATH,...]",
  );
  process.exitCode = 2;
}

if (process.argv[1] && resolve(process.argv[1]) === scriptPath) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 2;
  }
}
