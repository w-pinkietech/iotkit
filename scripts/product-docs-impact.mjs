#!/usr/bin/env node

/**
 * Product-docs impact selector and freshness soft gate.
 *
 * Authority: docs/product/
 * Format:    OKF v0.2 packaging
 * Gate:      scripts/check-product-docs.mjs (iotkit-product profile)
 *
 * Roles:
 * - select: lower-bound path → candidate product docs for freshness review
 * - soft-check: non-blocking CI warning when impact exists but neither
 *   docs/product nor a PR no-update reason is present (issue #165)
 * - form check remains scripts/check-product-docs.mjs
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

const SOFT_GATE_NOT_HARD =
  "This freshness check is a soft warning only. It never fails the job and is not a merge blocker. Hard gating is out of scope for issue #165.";

function normalizedPath(value) {
  return String(value).replaceAll("\\", "/").replace(/^\.\/+/, "");
}

/**
 * Normalize PR body text for template scanning only (not HTML rendering).
 * Drop full-line HTML comments used as PR-template placeholders.
 */
export function stripPrBodyNoise(body) {
  return String(body ?? "")
    .replace(/\r\n/g, "\n")
    .split("\n")
    .filter((line) => {
      const trimmed = line.trim();
      // Template placeholders are single-line <!-- ... --> comments.
      if (trimmed.startsWith("<!--") && trimmed.endsWith("-->")) {
        return false;
      }
      return true;
    })
    .join("\n");
}

/**
 * Detect a filled "No product-docs update reason" (or JA equivalent) in a PR body.
 * Intentionally minimal: a non-empty line after the template label, not a placeholder.
 */
export function hasNoProductDocsReason(prBody) {
  const body = stripPrBodyNoise(prBody);
  if (!body.trim()) {
    return false;
  }

  const labelPatterns = [
    /No product-docs update reason\s*\/\s*更新しない理由\s*:/i,
    /No product-docs update reason\s*:/i,
    /更新しない理由\s*:/,
  ];

  for (const label of labelPatterns) {
    const match = body.match(label);
    if (!match || match.index === undefined) {
      continue;
    }
    const after = body.slice(match.index + match[0].length);
    // Take the remainder of the section until the next markdown heading or ## section.
    const section = after.split(/\n##\s+/)[0] ?? after;
    for (const line of section.split("\n")) {
      const trimmed = line.trim();
      if (!trimmed) {
        continue;
      }
      // Bullet label lines without content, pure placeholders.
      if (/^[-*]\s*$/.test(trimmed)) {
        continue;
      }
      if (/^<!--/.test(trimmed)) {
        continue;
      }
      // A bullet that only repeats "none" without explanation is not enough.
      if (/^[-*]\s*none\s*$/i.test(trimmed)) {
        continue;
      }
      // Any other non-empty content counts as a recorded reason.
      return true;
    }
  }

  // Also accept an explicit one-line form used in short PR bodies.
  if (
    /no product-docs update reason\s*[:：]\s*\S+/i.test(body) ||
    /更新しない理由\s*[:：]\s*\S+/.test(body)
  ) {
    // Re-check that the trailing token is not only a placeholder comment remnant.
    const oneLine = body.match(
      /(?:no product-docs update reason|更新しない理由)\s*[:：]\s*(.+)/i,
    );
    if (oneLine?.[1]) {
      const value = oneLine[1].trim();
      if (value && !/^none$/i.test(value) && !value.startsWith("<!--")) {
        return true;
      }
    }
  }

  return false;
}

export function hasProductDocPathChanges(paths = []) {
  return paths.some((path) => {
    const normalized = normalizedPath(path);
    return (
      normalized.startsWith("docs/product/") && normalized.endsWith(".md")
    );
  });
}

/**
 * Soft freshness evaluation (non-blocking policy).
 *
 * @returns {{
 *   status: 'ok' | 'warn',
 *   code: string,
 *   impact: ReturnType<typeof selectImpact>,
 *   hasProductDocChanges: boolean,
 *   hasNoUpdateReason: boolean,
 *   message: string,
 * }}
 */
export function evaluateFreshnessSoft({ paths = [], prBody = "", rules }) {
  const impact = selectImpact(rules, paths);
  const hasProductDocChanges = hasProductDocPathChanges(paths);
  const hasNoUpdateReason = hasNoProductDocsReason(prBody);
  const hasImpactCandidates = impact.candidates.length > 0;

  if (!hasImpactCandidates) {
    return {
      status: "ok",
      code: "no_impact_candidates",
      impact,
      hasProductDocChanges,
      hasNoUpdateReason,
      message:
        "No impact-rule candidates for this change set. Soft gate does not warn (empty selection is still not a safety proof).",
    };
  }

  if (hasProductDocChanges) {
    return {
      status: "ok",
      code: "product_docs_updated",
      impact,
      hasProductDocChanges,
      hasNoUpdateReason,
      message:
        "Impact candidates exist and docs/product markdown is in the change set.",
    };
  }

  if (hasNoUpdateReason) {
    return {
      status: "ok",
      code: "no_update_reason_recorded",
      impact,
      hasProductDocChanges,
      hasNoUpdateReason,
      message:
        "Impact candidates exist and the PR body records a no product-docs update reason.",
    };
  }

  const candidateSummary = impact.candidates
    .slice(0, 8)
    .map((c) => c.fullPaths.join(" + "))
    .join("; ");
  const more =
    impact.candidates.length > 8
      ? ` (+${impact.candidates.length - 8} more)`
      : "";

  return {
    status: "warn",
    code: "missing_docs_and_reason",
    impact,
    hasProductDocChanges,
    hasNoUpdateReason,
    message:
      `Product-docs impact candidates exist, but this change set has neither docs/product markdown updates nor a filled "No product-docs update reason" in the PR body. ` +
      `Candidates: ${candidateSummary}${more}. ` +
      `Update matching docs/product files (ja+en) or record a concrete no-update reason. ${SOFT_GATE_NOT_HARD}`,
  };
}

export function formatSoftCheck(result) {
  const lines = [];
  lines.push("Product-docs freshness soft check");
  lines.push(SOFT_GATE_NOT_HARD);
  lines.push(`Result: ${result.status.toUpperCase()} (${result.code})`);
  lines.push(result.message);
  lines.push(
    `Impact candidates: ${result.impact.candidates.length}; ` +
      `docs/product changes: ${result.hasProductDocChanges}; ` +
      `no-update reason: ${result.hasNoUpdateReason}`,
  );
  if (result.impact.candidates.length > 0) {
    lines.push("Candidates:");
    for (const candidate of result.impact.candidates.slice(0, 12)) {
      lines.push(
        `- ${candidate.fullPaths.join(" + ")} (rules: ${candidate.ruleIds.join(", ")})`,
      );
    }
  }
  lines.push("");
  lines.push(EMPTY_IS_NOT_SAFE);
  return lines.join("\n");
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
  let prBody = "";
  let prBodyFile = "";
  let prBodyEnv = "";
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index];
    const value = args[index + 1];
    if (argument === "--base" && value) {
      base = value;
      index += 1;
    } else if (argument === "--paths" && value) {
      paths.push(...value.split(",").filter(Boolean));
      index += 1;
    } else if (argument === "--pr-body" && value) {
      prBody = value;
      index += 1;
    } else if (argument === "--pr-body-file" && value) {
      prBodyFile = value;
      index += 1;
    } else if (argument === "--pr-body-env" && value) {
      prBodyEnv = value;
      index += 1;
    } else {
      throw new Error(`unknown or incomplete argument: ${argument}`);
    }
  }
  if (!base && paths.length === 0) {
    throw new Error("select/soft-check requires --base REF or --paths PATH[,PATH]");
  }
  return { base, paths, prBody, prBodyFile, prBodyEnv };
}

function resolvePrBody({ prBody, prBodyFile, prBodyEnv }) {
  if (prBodyFile) {
    return readFileSync(prBodyFile, "utf8");
  }
  if (prBodyEnv) {
    return process.env[prBodyEnv] ?? "";
  }
  return prBody;
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
      "usage: node scripts/product-docs-impact.mjs check | select ... | soft-check ...",
      "",
      "check       Validate impact rules against the repository product corpus.",
      "select      List candidate docs/product paths for a change set.",
      "soft-check  Non-blocking freshness warning when impact exists without",
      "            docs/product changes or a PR no-update reason (issue #165).",
      "",
      EMPTY_IS_NOT_SAFE,
      SOFT_GATE_NOT_HARD,
      "",
      "Examples:",
      "  node scripts/product-docs-impact.mjs select --base origin/master",
      "  node scripts/product-docs-impact.mjs select --paths edge-node/ingest/http/src/lib.rs",
      "  node scripts/product-docs-impact.mjs soft-check --base origin/master --pr-body-env PR_BODY",
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

  if (command === "soft-check") {
    const selectionArgs = parseSelectArgs(args.slice(1));
    const paths = selectionArgs.base
      ? [
          ...new Set([
            ...changedPaths(defaultRepoRoot, selectionArgs.base),
            ...selectionArgs.paths,
          ]),
        ]
      : selectionArgs.paths;
    const prBody = resolvePrBody(selectionArgs);
    const result = evaluateFreshnessSoft({ paths, prBody, rules });
    const text = formatSoftCheck(result);
    console.log(text);
    if (result.status === "warn") {
      // Always non-blocking: warn annotation for Actions UI, exit 0.
      if (process.env.GITHUB_ACTIONS === "true") {
        const oneLine = result.message.replace(/\s+/g, " ").slice(0, 900);
        console.log(`::warning title=Product-docs freshness::${oneLine}`);
      }
    }
    // Soft gate never fails the process (hard gate is a later issue).
    process.exitCode = 0;
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
