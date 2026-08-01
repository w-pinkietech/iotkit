#!/usr/bin/env node

/**
 * Product-docs impact selector.
 *
 * Authority: docs/product/
 * Format:    OKF v0.2 packaging
 * Gate:      scripts/check-product-docs.mjs (iotkit-product profile)
 *
 * Role: lower-bound path → candidate product docs for freshness review.
 * Not a form check (see check-product-docs.mjs) and not a CI soft gate (#165).
 */

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptPath = fileURLToPath(import.meta.url);
const defaultRepoRoot = resolve(dirname(scriptPath), "..");
const rulesRelativePath = "scripts/docs/product-docs-impact-rules.json";
const productDocLocales = ["en", "ja"];
const forbiddenPrefixes = new Set(["", ".", "./", "*", "**", "**/*"]);

const EMPTY_IS_NOT_SAFE =
  "Empty selection is not proof that product docs need no update. Semantic or operator-visible changes still require a human judgment (update docs/product/ or record a concrete no-update reason on the PR).";

function normalizedPath(value) {
  return String(value).replaceAll("\\", "/").replace(/^\.\/+/, "");
}

export function loadRules(repoRoot = defaultRepoRoot) {
  return JSON.parse(
    readFileSync(resolve(repoRoot, rulesRelativePath), "utf8"),
  );
}

function nonEmptyStrings(value) {
  return (
    Array.isArray(value) &&
    value.length > 0 &&
    value.every((item) => typeof item === "string" && item.trim() !== "")
  );
}

function pathMatches(path, prefix) {
  const normalized = normalizedPath(prefix);
  if (normalized.endsWith("/")) {
    return path.startsWith(normalized);
  }
  // Allow file prefixes used as directory-like stems (e.g. scripts/iotkit).
  return path === normalized || path.startsWith(`${normalized}/`) || path.startsWith(normalized);
}

/**
 * Validate impact rules. Returns error strings (empty = OK).
 */
export function validateRules(rules, repoRoot = defaultRepoRoot) {
  const errors = [];
  if (rules?.schema_version !== 1) {
    errors.push("schema_version must be 1");
  }
  if (!Array.isArray(rules?.rules) || rules.rules.length === 0) {
    errors.push("rules must be a non-empty array");
    return errors;
  }

  const ids = new Set();
  for (const [index, rule] of rules.rules.entries()) {
    const location = `rules[${index}]`;
    if (typeof rule?.id !== "string" || rule.id.trim() === "") {
      errors.push(`${location}.id must be a non-empty string`);
    } else if (ids.has(rule.id)) {
      errors.push(`${location}.id duplicates ${rule.id}`);
    } else {
      ids.add(rule.id);
    }

    if (!nonEmptyStrings(rule?.path_prefixes)) {
      errors.push(`${location}.path_prefixes must be non-empty strings`);
    } else {
      for (const prefix of rule.path_prefixes) {
        const normalized = normalizedPath(prefix);
        if (forbiddenPrefixes.has(normalized)) {
          errors.push(`${location}.path_prefixes contains catch-all ${prefix}`);
        }
      }
    }

    if (!Array.isArray(rule?.doc_paths)) {
      errors.push(`${location}.doc_paths must be an array`);
    } else {
      for (const docPath of rule.doc_paths) {
        if (typeof docPath !== "string" || docPath.trim() === "") {
          errors.push(`${location}.doc_paths contains an empty path`);
          continue;
        }
        const relative = normalizedPath(docPath);
        if (relative.startsWith("docs/product/")) {
          errors.push(
            `${location}.doc_paths must be relative to docs/product/<lang>/, not absolute: ${docPath}`,
          );
          continue;
        }
        for (const locale of productDocLocales) {
          const full = resolve(repoRoot, "docs/product", locale, relative);
          if (!existsSync(full)) {
            errors.push(
              `${location}.doc_paths missing docs/product/${locale}/${relative}`,
            );
          }
        }
      }
    }

    if (typeof rule?.rationale !== "string" || rule.rationale.trim() === "") {
      errors.push(`${location}.rationale must be a non-empty string`);
    }
  }
  return errors;
}

/**
 * @param {object} rules
 * @param {string[]} paths
 * @returns {{
 *   candidates: Array<{ docPath: string, locales: string[], fullPaths: string[], ruleIds: string[], matchedPaths: string[], rationales: string[] }>,
 *   matchedRules: Array<{ rule: object, matchedPaths: string[] }>,
 *   unmatchedPaths: string[],
 *   flags: string[],
 *   alreadyTouchedProductDocs: string[],
 * }}
 */
export function selectImpact(rules, paths = []) {
  const normalizedPaths = [
    ...new Set(paths.map(normalizedPath).filter(Boolean)),
  ];
  const matchedRules = [];
  const candidateMap = new Map();
  const flags = new Set();

  for (const rule of rules.rules) {
    const matchedPaths = normalizedPaths.filter((path) =>
      rule.path_prefixes.some((prefix) => pathMatches(path, prefix)),
    );
    if (matchedPaths.length === 0) {
      continue;
    }
    matchedRules.push({ rule, matchedPaths });
    for (const flag of rule.flags ?? []) {
      flags.add(flag);
    }
    for (const docPath of rule.doc_paths ?? []) {
      const key = normalizedPath(docPath);
      let entry = candidateMap.get(key);
      if (!entry) {
        entry = {
          docPath: key,
          locales: [...productDocLocales],
          fullPaths: productDocLocales.map(
            (locale) => `docs/product/${locale}/${key}`,
          ),
          ruleIds: [],
          matchedPaths: [],
          rationales: [],
        };
        candidateMap.set(key, entry);
      }
      if (!entry.ruleIds.includes(rule.id)) {
        entry.ruleIds.push(rule.id);
      }
      if (!entry.rationales.includes(rule.rationale)) {
        entry.rationales.push(rule.rationale);
      }
      for (const path of matchedPaths) {
        if (!entry.matchedPaths.includes(path)) {
          entry.matchedPaths.push(path);
        }
      }
    }
  }

  const candidates = [...candidateMap.values()].sort((a, b) =>
    a.docPath.localeCompare(b.docPath),
  );

  const unmatchedPaths = normalizedPaths.filter(
    (path) =>
      !rules.rules.some((rule) =>
        rule.path_prefixes.some((prefix) => pathMatches(path, prefix)),
      ),
  );

  const alreadyTouchedProductDocs = normalizedPaths.filter((path) =>
    path.startsWith("docs/product/") && path.endsWith(".md"),
  );

  return {
    candidates,
    matchedRules,
    unmatchedPaths,
    flags: [...flags].sort(),
    alreadyTouchedProductDocs,
  };
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

export function changedPaths(repoRoot, base) {
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
      ...gitLines(repoRoot, ["ls-files", "--others", "--exclude-standard"]),
    ]),
  ];
}

function parseSelectArgs(args) {
  let base = "";
  const paths = [];
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index];
    const value = args[index + 1];
    if (argument === "--base" && value) {
      base = value;
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
  return { base, paths };
}

function limited(values, limit = 5) {
  const visible = values.slice(0, limit);
  const remainder = values.length - visible.length;
  return remainder > 0
    ? `${visible.join(", ")}, and ${remainder} more`
    : visible.join(", ");
}

export function formatSelection(result, paths) {
  const lines = [];
  lines.push("Product-docs impact selection");
  lines.push("Authority: docs/product/ | Format: OKF v0.2 | Gate: check-product-docs (iotkit-product)");
  lines.push(`Changed paths: ${paths.length}`);
  lines.push(`Candidate product docs: ${result.candidates.length}`);

  if (result.candidates.length === 0) {
    lines.push("Candidates: none");
  }

  for (const candidate of result.candidates) {
    lines.push("");
    lines.push(`- ${candidate.fullPaths.join(" + ")}`);
    lines.push(`  Rules: ${candidate.ruleIds.join(", ")}`);
    lines.push(`  Matched paths: ${limited(candidate.matchedPaths)}`);
    lines.push(`  Why: ${candidate.rationales.join(" ")}`);
  }

  if (result.alreadyTouchedProductDocs.length > 0) {
    lines.push("");
    lines.push("Product docs already in the change set:");
    for (const path of result.alreadyTouchedProductDocs.slice(0, 30)) {
      lines.push(`- ${path}`);
    }
    if (result.alreadyTouchedProductDocs.length > 30) {
      lines.push(
        `- and ${result.alreadyTouchedProductDocs.length - 30} more`,
      );
    }
  }

  if (result.flags.includes("corpus_touched") && result.candidates.length === 0) {
    lines.push("");
    lines.push(
      "Note: docs/product/ is already touched; confirm bilingual pairs and shared revision bumps, then run node scripts/check-product-docs.mjs.",
    );
  }

  if (result.unmatchedPaths.length > 0) {
    lines.push("");
    lines.push("Unmatched paths (still may need product-doc judgment):");
    for (const path of result.unmatchedPaths.slice(0, 20)) {
      lines.push(`- ${path}`);
    }
    if (result.unmatchedPaths.length > 20) {
      lines.push(`- and ${result.unmatchedPaths.length - 20} more`);
    }
  }

  lines.push("");
  lines.push(EMPTY_IS_NOT_SAFE);
  lines.push(
    "After updates: edit ja+en together, bump shared revision when concept content changes, run node scripts/check-product-docs.mjs.",
  );
  return lines.join("\n");
}

function printHelp(stream = console.error) {
  stream(
    [
      "usage: node scripts/product-docs-impact.mjs check | select --base REF [--paths PATH,...] | select --paths PATH[,PATH]",
      "",
      "check   Validate impact rules against the repository product corpus.",
      "select  List candidate docs/product paths for a change set.",
      "",
      EMPTY_IS_NOT_SAFE,
      "",
      "Examples:",
      "  node scripts/product-docs-impact.mjs select --base origin/master",
      "  node scripts/product-docs-impact.mjs select --paths edge-node/ingest/http/src/lib.rs",
      "  node scripts/product-docs-impact.mjs check",
    ].join("\n"),
  );
}

function main(args) {
  const command = args[0];
  const rules = loadRules();
  const errors = validateRules(rules);
  if (errors.length > 0) {
    for (const error of errors) {
      console.error(`product-docs impact rules: ${error}`);
    }
    process.exitCode = 1;
    return;
  }

  if (command === "check") {
    console.log(
      `product-docs impact: OK (${rules.rules.length} rules, all doc targets exist)`,
    );
    return;
  }

  if (command === "select") {
    const selectionArgs = parseSelectArgs(args.slice(1));
    const paths = selectionArgs.base
      ? [
          ...new Set([
            ...changedPaths(defaultRepoRoot, selectionArgs.base),
            ...selectionArgs.paths,
          ]),
        ]
      : selectionArgs.paths;
    const result = selectImpact(rules, paths);
    console.log(formatSelection(result, paths));
    return;
  }

  if (command === "help" || command === "--help" || command === "-h") {
    printHelp(console.log);
    return;
  }

  printHelp();
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
