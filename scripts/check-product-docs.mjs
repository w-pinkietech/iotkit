#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { execFileSync } from "node:child_process";
import { parseFrontmatterContent, validateRequiredScalars } from "./docs/frontmatter.mjs";

const repoRoot = path.resolve(import.meta.dirname, "..");
const bundleRoot = path.join(repoRoot, "docs", "product");
const locales = ["ja", "en"];
const allowedTypes = new Set(["Concept", "Architecture", "Contract", "Runbook"]);
const allowedStatuses = new Set(["draft", "stable", "deprecated"]);
const allowedCategories = new Set(["concepts", "architecture", "contracts", "operations"]);
const errors = [];

/** @type {"okf-min" | "iotkit-product" | "all"} */
function resolveMode() {
  const fromArg = process.argv.find((arg) => arg.startsWith("--mode="))?.slice("--mode=".length);
  const raw = fromArg || process.env.PRODUCT_DOCS_MODE || "all";
  if (raw === "okf-min" || raw === "iotkit-product" || raw === "all") return raw;
  console.error(`Unknown mode "${raw}". Use okf-min | iotkit-product | all.`);
  process.exit(2);
}

const mode = resolveMode();
const runOkfMin = mode === "okf-min" || mode === "all";
const runIotkit = mode === "iotkit-product" || mode === "all";
const packagingLayer = runOkfMin ? "okf-min" : "iotkit-product";

function fail(file, message, layer = "iotkit-product") {
  errors.push({ layer, text: `${path.relative(repoRoot, file)}: ${message}` });
}

function reportFailuresAndExit() {
  const byLayer = { "okf-min": [], "iotkit-product": [] };
  for (const error of errors) byLayer[error.layer]?.push(error.text);
  console.error(`Product docs validation failed (mode=${mode}, ${errors.length} issue(s)):`);
  for (const layer of ["okf-min", "iotkit-product"]) {
    if (byLayer[layer].length === 0) continue;
    console.error(`[${layer}]`);
    for (const text of byLayer[layer]) console.error(`- ${text}`);
  }
  process.exit(1);
}

function bundleFiles(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const target = path.join(directory, entry.name);
    const stats = fs.lstatSync(target);
    if (stats.isSymbolicLink()) {
      fail(target, "symbolic links are not allowed in the portable bundle", packagingLayer);
      return [];
    }
    if (entry.isDirectory()) return bundleFiles(target);
    if (!entry.isFile() || !entry.name.endsWith(".md")) {
      fail(target, "only Markdown files are allowed in the current bundle profile", packagingLayer);
      return [];
    }
    return [target];
  });
}

function parseFrontmatter(file, content, layer = "okf-min") {
  const result = parseFrontmatterContent(content);
  if (result.error) {
    fail(file, result.error, layer);
    return null;
  }
  return result;
}

function linksFrom(content) {
  const links = [];
  const regex = /!?\[[^\]]*\]\(([^)\s]+)(?:\s+["'][^"']*["'])?\)/g;
  for (const match of content.matchAll(regex)) links.push(match[1]);
  return links;
}

function isMovingGitHubMasterLink(href) {
  return /^https?:\/\/github\.com\/[^/?#]+\/[^/?#]+\/(?:blob|tree)\/master(?:[/?#]|$)/i.test(href);
}

function localMarkdownTarget(from, href) {
  const raw = href.split("#", 1)[0];
  let withoutFragment;
  try {
    withoutFragment = decodeURIComponent(raw);
  } catch {
    fail(from, `local link is not a valid URI reference: ${href}`, "iotkit-product");
    return null;
  }
  if (!withoutFragment || /^[a-z][a-z0-9+.-]*:/i.test(withoutFragment)) return null;
  const candidate = withoutFragment.startsWith("/")
    ? path.join(bundleRoot, withoutFragment.slice(1))
    : path.resolve(path.dirname(from), withoutFragment);
  if (fs.existsSync(candidate) && fs.statSync(candidate).isDirectory()) return path.join(candidate, "index.md");
  return candidate;
}

function inside(directory, target) {
  const relative = path.relative(directory, target);
  return relative === "" || (!relative.startsWith(`..${path.sep}`) && relative !== "..");
}

if (!fs.existsSync(bundleRoot)) {
  fail(bundleRoot, "does not exist", packagingLayer);
  reportFailuresAndExit();
}

const rootIndex = path.join(bundleRoot, "index.md");
if (!fs.existsSync(rootIndex)) {
  fail(rootIndex, "does not exist", packagingLayer);
  reportFailuresAndExit();
}
const root = parseFrontmatter(rootIndex, fs.readFileSync(rootIndex, "utf8"), packagingLayer);
if (runOkfMin) {
  if (!root || root.metadata.okf_version !== "0.2") {
    fail(rootIndex, 'bundle root must declare okf_version: "0.2"', "okf-min");
  }
}
if (runIotkit && root && Object.keys(root.metadata).some((key) => key !== "okf_version")) {
  fail(rootIndex, "bundle root index may only declare okf_version", "iotkit-product");
}

const concepts = new Map();
for (const file of bundleFiles(bundleRoot)) {
  const relative = path.relative(bundleRoot, file);
  const content = fs.readFileSync(file, "utf8");
  const basename = path.basename(file);
  if (basename === "log.md") {
    if (runIotkit) {
      fail(file, "log.md is not supported by the current IoTKit product-docs producer profile", "iotkit-product");
    }
    // okf-min: log.md is a reserved name; skip concept rules.
  } else if (basename === "index.md") {
    if (file !== rootIndex && /^---\r?\n/.test(content)) {
      fail(
        file,
        "reserved index files must not have concept frontmatter",
        packagingLayer,
      );
    }
  } else {
    const parsed = parseFrontmatter(file, content, packagingLayer);
    if (!parsed) continue;
    const { metadata } = parsed;
    if (runOkfMin) {
      const type = metadata.type;
      if (type === undefined || type === null || type === "") {
        fail(file, "missing required field type", "okf-min");
      } else if (typeof type !== "string" || !type.trim()) {
        fail(file, "type must be a non-empty string", "okf-min");
      }
    }
    if (runIotkit) {
      for (const message of validateRequiredScalars(metadata)) fail(file, message, "iotkit-product");
      if (metadata.type && !allowedTypes.has(metadata.type)) {
        fail(file, `unsupported type ${metadata.type}`, "iotkit-product");
      }
      if (metadata.status && !allowedStatuses.has(metadata.status)) {
        fail(file, `unsupported status ${metadata.status}`, "iotkit-product");
      }
      const [locale, category] = relative.split(path.sep);
      if (!locales.includes(locale)) fail(file, "concept must be below ja/ or en/", "iotkit-product");
      if (!allowedCategories.has(category)) {
        fail(file, `unsupported top-level category ${category ?? "<missing>"}`, "iotkit-product");
      }
      if (metadata.language !== locale) {
        fail(file, `language ${metadata.language ?? "<missing>"} does not match path ${locale}`, "iotkit-product");
      }
      const key = `${locale}:${metadata.translation_key}`;
      if (concepts.has(key)) fail(file, `duplicate translation_key ${metadata.translation_key}`, "iotkit-product");
      concepts.set(key, { file, relative: relative.slice(locale.length + 1), metadata });
    }
  }
  if (runIotkit) {
    for (const href of linksFrom(content)) {
      if (isMovingGitHubMasterLink(href)) {
        fail(file, `moving GitHub master link is not release evidence: ${href}`, "iotkit-product");
        continue;
      }
      const target = localMarkdownTarget(file, href);
      if (target && !inside(bundleRoot, target)) fail(file, `local link escapes the bundle: ${href}`, "iotkit-product");
      else if (target && !fs.existsSync(target)) fail(file, `broken local link ${href}`, "iotkit-product");
    }
  }
}

const translationKeys = new Set([...concepts.values()].map(({ metadata }) => metadata.translation_key).filter(Boolean));
const baseRef = process.env.OKF_BASE_REF;
if (runIotkit && baseRef) {
  const previousPath = new Map();
  const baseConceptsByPath = new Map();
  const baseConceptsByKey = new Map();
  const contentChanged = new Set();
  try {
    const baseMarkdownFiles = execFileSync(
      "git",
      ["ls-tree", "-r", "--name-only", baseRef, "--", "docs/product", "docs/okf"],
      { cwd: repoRoot, encoding: "utf8" },
    )
      .trim()
      .split(/\r?\n/)
      .filter(
        (file) =>
          file.endsWith(".md") &&
          !file.endsWith("/index.md") &&
          !file.endsWith("/log.md") &&
          (file.startsWith("docs/product/") || file.startsWith("docs/okf/ja/") || file.startsWith("docs/okf/en/")),
      );
    const baseHasProductCorpus = baseMarkdownFiles.some((file) => file.startsWith("docs/product/"));
    const baseFiles = baseMarkdownFiles.filter((file) =>
      baseHasProductCorpus ? file.startsWith("docs/product/") : file.startsWith("docs/okf/"),
    );

    const changes = execFileSync(
      "git",
      ["diff", "--name-status", "-M", `${baseRef}...HEAD`, "--", "docs/product", "docs/okf"],
      { cwd: repoRoot, encoding: "utf8" },
    )
      .trim()
      .split(/\r?\n/)
      .filter(Boolean);

    for (const line of changes) {
      const [status, first, second] = line.split("\t");
      // Once docs/product exists in the base, docs/okf files are forwarding
      // stubs and must not drive product-document revision checks.
      if (baseHasProductCorpus && first?.startsWith("docs/okf/")) continue;
      if (status.startsWith("R") || status.startsWith("C")) {
        const dest = second.replace(/^docs\/okf\//, "docs/product/");
        // Ignore the compatibility stub at docs/okf/index.md.
        if (!dest.startsWith("docs/product/")) continue;
        const knownPrevious = previousPath.get(dest);
        if (!knownPrevious || first.startsWith("docs/okf/")) previousPath.set(dest, first);
        const similarity = Number.parseInt(status.slice(1) || "0", 10);
        if (!(status.startsWith("R") && similarity === 100)) contentChanged.add(dest);
      } else if (first) {
        if (first === "docs/okf/index.md") continue;
        const dest = first.replace(/^docs\/okf\//, "docs/product/");
        if (!dest.startsWith("docs/product/")) continue;
        const knownPrevious = previousPath.get(dest);
        if (!knownPrevious || first.startsWith("docs/okf/")) previousPath.set(dest, first);
        // Added/modified paths may still be renames Git did not pair; compare blobs below.
        if (status === "M" || status === "A") contentChanged.add(dest);
      }
    }

    for (const file of baseFiles) {
      const previous = execFileSync("git", ["show", `${baseRef}:${file}`], { cwd: repoRoot, encoding: "utf8" });
      const parsed = parseFrontmatter(path.join(repoRoot, file), previous, "iotkit-product");
      if (parsed) {
        const productPath = file.replace(/^docs\/okf\//, "docs/product/");
        baseConceptsByPath.set(file, parsed.metadata);
        baseConceptsByPath.set(productPath, parsed.metadata);
        baseConceptsByKey.set(`${parsed.metadata.language}:${parsed.metadata.translation_key}`, parsed.metadata);
        if (!previousPath.has(productPath)) previousPath.set(productPath, file);
      }
    }

    function blobDiffers(oldPath, newPath) {
      try {
        execFileSync("git", ["diff", "--quiet", `${baseRef}:${oldPath}`, `HEAD:${newPath}`], {
          cwd: repoRoot,
          stdio: "ignore",
        });
        return false;
      } catch (error) {
        return true;
      }
    }

    // Drop pure renames / identical content from contentChanged.
    for (const dest of [...contentChanged]) {
      if (!dest.endsWith(".md") || dest.endsWith("/index.md")) {
        contentChanged.delete(dest);
        continue;
      }
      const oldPath = previousPath.get(dest) ?? dest.replace(/^docs\/product\//, "docs/okf/");
      if (!blobDiffers(oldPath, dest)) contentChanged.delete(dest);
    }
  } catch (error) {
    fail(rootIndex, `cannot compare translations with base ${baseRef}: ${error.message}`, "iotkit-product");
  }

  for (const translationKey of translationKeys) {
    const ja = concepts.get(`ja:${translationKey}`);
    const en = concepts.get(`en:${translationKey}`);
    if (!ja || !en) continue;
    const jaPath = path.relative(repoRoot, ja.file).split(path.sep).join("/");
    const enPath = path.relative(repoRoot, en.file).split(path.sep).join("/");
    // Pure path renames alone do not require paired content edits or revision bumps.
    if (!contentChanged.has(jaPath) && !contentChanged.has(enPath)) continue;
    if (!contentChanged.has(jaPath) || !contentChanged.has(enPath)) {
      fail(ja.file, `both translations must change together for ${translationKey}`, "iotkit-product");
      continue;
    }
    for (const concept of [ja, en]) {
      const filePath = path.relative(repoRoot, concept.file).split(path.sep).join("/");
      const oldMetadata =
        baseConceptsByPath.get(previousPath.get(filePath) ?? filePath) ??
        baseConceptsByKey.get(`${concept.metadata.language}:${concept.metadata.translation_key}`);
      if (!oldMetadata) continue;
      if (oldMetadata.language !== concept.metadata.language) {
        fail(concept.file, `language is immutable and was ${oldMetadata.language}`, "iotkit-product");
      }
      if (oldMetadata.translation_key !== concept.metadata.translation_key) {
        fail(concept.file, `translation_key is immutable and was ${oldMetadata.translation_key}`, "iotkit-product");
      }
      const currentRevision = concept.metadata.revision;
      const previousRevision = oldMetadata.revision;
      if (!/^[1-9][0-9]*$/.test(currentRevision ?? "")) continue;
      if (!/^[1-9][0-9]*$/.test(previousRevision ?? "")) {
        fail(
          concept.file,
          `base revision is not a positive integer: ${previousRevision ?? "<missing>"}`,
          "iotkit-product",
        );
        continue;
      }
      if (BigInt(currentRevision) <= BigInt(previousRevision)) {
        fail(concept.file, `revision must increase from the base version ${oldMetadata.revision}`, "iotkit-product");
      }
    }
  }
}

if (runIotkit) {
  for (const translationKey of translationKeys) {
    const ja = concepts.get(`ja:${translationKey}`);
    const en = concepts.get(`en:${translationKey}`);
    if (!ja || !en) {
      fail((ja ?? en).file, `translation_key ${translationKey} must have one ja and one en document`, "iotkit-product");
      continue;
    }
    if (ja.relative !== en.relative) fail(ja.file, `translation path differs from ${en.relative}`, "iotkit-product");
    for (const field of ["type", "status", "revision"]) {
      if (ja.metadata[field] !== en.metadata[field]) {
        fail(ja.file, `${field} differs from English translation`, "iotkit-product");
      }
    }
  }

  const reachable = new Set();
  const queue = [rootIndex];
  while (queue.length > 0) {
    const file = queue.shift();
    if (reachable.has(file) || !fs.existsSync(file)) continue;
    reachable.add(file);
    for (const href of linksFrom(fs.readFileSync(file, "utf8"))) {
      const target = localMarkdownTarget(file, href);
      if (target && inside(bundleRoot, target) && target.endsWith(".md")) queue.push(target);
    }
  }
  for (const locale of locales) {
    const localeIndex = path.join(bundleRoot, locale, "index.md");
    if (!reachable.has(localeIndex)) {
      fail(localeIndex, "locale index is not reachable from bundle root index.md", "iotkit-product");
    }
  }
  for (const { file } of concepts.values()) {
    if (!reachable.has(file)) fail(file, "not reachable from bundle root index.md", "iotkit-product");
  }
}

if (errors.length > 0) {
  reportFailuresAndExit();
}

const summary =
  mode === "okf-min"
    ? "OKF min packaging checks passed."
    : `Product docs (IoTKit producer profile; OKF v0.2 packaging) validation passed: ${translationKeys.size} bilingual concepts.`;
console.log(`[mode=${mode}] ${summary}`);
