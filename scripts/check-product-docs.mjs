#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { execFileSync } from "node:child_process";

const repoRoot = path.resolve(import.meta.dirname, "..");
const bundleRoot = path.join(repoRoot, "docs", "product");
const locales = ["ja", "en"];
const required = ["type", "title", "description", "language", "translation_key", "status", "revision"];
const allowedTypes = new Set(["Concept", "Architecture", "Contract", "Runbook"]);
const allowedStatuses = new Set(["draft", "stable", "deprecated"]);
const allowedCategories = new Set(["concepts", "architecture", "contracts", "operations"]);
const plainStringFields = new Set(["type", "language", "translation_key", "status"]);
const errors = [];

function fail(file, message) {
  errors.push(`${path.relative(repoRoot, file)}: ${message}`);
}

function bundleFiles(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const target = path.join(directory, entry.name);
    const stats = fs.lstatSync(target);
    if (stats.isSymbolicLink()) {
      fail(target, "symbolic links are not allowed in the portable bundle");
      return [];
    }
    if (entry.isDirectory()) return bundleFiles(target);
    if (!entry.isFile() || !entry.name.endsWith(".md")) {
      fail(target, "only Markdown files are allowed in the current bundle profile");
      return [];
    }
    return [target];
  });
}

function scalar(raw, key) {
  const value = raw.trim();
  if (key === "revision") return /^[1-9][0-9]*$/.test(value) ? value : null;
  if (plainStringFields.has(key)) {
    return /^[A-Za-z0-9][A-Za-z0-9 ._-]*$/.test(value) ? value : null;
  }
  if (value.startsWith('"')) {
    try {
      const parsed = JSON.parse(value);
      return typeof parsed === "string" ? parsed : null;
    } catch {
      return null;
    }
  }
  return null;
}

function parseFrontmatter(file, content) {
  const match = content.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n/);
  if (!match) {
    fail(file, "missing YAML frontmatter");
    return null;
  }
  const metadata = {};
  for (const [index, line] of match[1].split(/\r?\n/).entries()) {
    if (!line.trim() || line.trimStart().startsWith("#")) continue;
    const field = line.match(/^([A-Za-z_][A-Za-z0-9_-]*):\s*(.+)$/);
    if (!field) {
      fail(file, `unsupported or invalid frontmatter at line ${index + 2}`);
      continue;
    }
    if (Object.hasOwn(metadata, field[1])) fail(file, `duplicate frontmatter field ${field[1]}`);
    const value = scalar(field[2], field[1]);
    if (value === null) {
      fail(file, `frontmatter value at line ${index + 2} is outside the IoTKit OKF scalar profile`);
      continue;
    }
    metadata[field[1]] = value;
  }
  return { metadata, body: content.slice(match[0].length) };
}

function linksFrom(content) {
  const links = [];
  const regex = /!?\[[^\]]*\]\(([^)\s]+)(?:\s+["'][^"']*["'])?\)/g;
  for (const match of content.matchAll(regex)) links.push(match[1]);
  return links;
}

function localMarkdownTarget(from, href) {
  const raw = href.split("#", 1)[0];
  let withoutFragment;
  try {
    withoutFragment = decodeURIComponent(raw);
  } catch {
    fail(from, `local link is not a valid URI reference: ${href}`);
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
  console.error("docs/product does not exist");
  process.exit(1);
}

const rootIndex = path.join(bundleRoot, "index.md");
if (!fs.existsSync(rootIndex)) {
  fail(rootIndex, "does not exist");
  console.error(`Product docs / OKF validation failed (${errors.length}):`);
  for (const error of errors) console.error(`- ${error}`);
  process.exit(1);
}
const root = parseFrontmatter(rootIndex, fs.readFileSync(rootIndex, "utf8"));
if (!root || root.metadata.okf_version !== "0.2") fail(rootIndex, 'bundle root must declare okf_version: "0.2"');
if (root && Object.keys(root.metadata).some((key) => key !== "okf_version")) {
  fail(rootIndex, "bundle root index may only declare okf_version");
}

const concepts = new Map();
for (const file of bundleFiles(bundleRoot)) {
  const relative = path.relative(bundleRoot, file);
  const content = fs.readFileSync(file, "utf8");
  const basename = path.basename(file);
  if (basename === "log.md") {
    fail(file, "log.md is not supported by the current IoTKit product-docs producer profile");
  } else if (basename === "index.md") {
    if (file !== rootIndex && /^---\r?\n/.test(content)) fail(file, "reserved index files must not have concept frontmatter");
  } else {
    const parsed = parseFrontmatter(file, content);
    if (!parsed) continue;
    const { metadata } = parsed;
    for (const key of required) if (!metadata[key]) fail(file, `missing required field ${key}`);
    if (metadata.type && !allowedTypes.has(metadata.type)) fail(file, `unsupported type ${metadata.type}`);
    if (metadata.status && !allowedStatuses.has(metadata.status)) fail(file, `unsupported status ${metadata.status}`);
    if (!/^[1-9][0-9]*$/.test(metadata.revision ?? "")) fail(file, "revision must be a positive integer");
    const [locale, category] = relative.split(path.sep);
    if (!locales.includes(locale)) fail(file, "concept must be below ja/ or en/");
    if (!allowedCategories.has(category)) fail(file, `unsupported top-level category ${category ?? "<missing>"}`);
    if (metadata.language !== locale) fail(file, `language ${metadata.language ?? "<missing>"} does not match path ${locale}`);
    const key = `${locale}:${metadata.translation_key}`;
    if (concepts.has(key)) fail(file, `duplicate translation_key ${metadata.translation_key}`);
    concepts.set(key, { file, relative: relative.slice(locale.length + 1), metadata });
  }
  for (const href of linksFrom(content)) {
    const target = localMarkdownTarget(file, href);
    if (target && !inside(bundleRoot, target)) fail(file, `local link escapes the bundle: ${href}`);
    else if (target && !fs.existsSync(target)) fail(file, `broken local link ${href}`);
  }
}

const translationKeys = new Set([...concepts.values()].map(({ metadata }) => metadata.translation_key).filter(Boolean));
const baseRef = process.env.OKF_BASE_REF;
if (baseRef) {
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
      const parsed = parseFrontmatter(path.join(repoRoot, file), previous);
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
    fail(rootIndex, `cannot compare translations with base ${baseRef}: ${error.message}`);
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
      fail(ja.file, `both translations must change together for ${translationKey}`);
      continue;
    }
    for (const concept of [ja, en]) {
      const filePath = path.relative(repoRoot, concept.file).split(path.sep).join("/");
      const oldMetadata =
        baseConceptsByPath.get(previousPath.get(filePath) ?? filePath) ??
        baseConceptsByKey.get(`${concept.metadata.language}:${concept.metadata.translation_key}`);
      if (!oldMetadata) continue;
      if (oldMetadata.language !== concept.metadata.language) {
        fail(concept.file, `language is immutable and was ${oldMetadata.language}`);
      }
      if (oldMetadata.translation_key !== concept.metadata.translation_key) {
        fail(concept.file, `translation_key is immutable and was ${oldMetadata.translation_key}`);
      }
      if (Number(concept.metadata.revision) <= Number(oldMetadata.revision)) {
        fail(concept.file, `revision must increase from the base version ${oldMetadata.revision}`);
      }
    }
  }
}

for (const translationKey of translationKeys) {
  const ja = concepts.get(`ja:${translationKey}`);
  const en = concepts.get(`en:${translationKey}`);
  if (!ja || !en) {
    fail((ja ?? en).file, `translation_key ${translationKey} must have one ja and one en document`);
    continue;
  }
  if (ja.relative !== en.relative) fail(ja.file, `translation path differs from ${en.relative}`);
  for (const field of ["type", "status", "revision"]) {
    if (ja.metadata[field] !== en.metadata[field]) fail(ja.file, `${field} differs from English translation`);
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
  if (!reachable.has(localeIndex)) fail(localeIndex, "locale index is not reachable from bundle root index.md");
}
for (const { file } of concepts.values()) {
  if (!reachable.has(file)) fail(file, "not reachable from bundle root index.md");
}

if (errors.length > 0) {
  console.error(
    `Product docs (IoTKit producer profile) validation failed (${errors.length}). ` +
      `This is the repository product gate, not plain OKF consumer tolerance:`,
  );
  for (const error of errors) console.error(`- ${error}`);
  process.exit(1);
}
console.log(
  `Product docs (IoTKit producer profile; OKF v0.2 packaging) validation passed: ` +
    `${translationKeys.size} bilingual concepts.`,
);
