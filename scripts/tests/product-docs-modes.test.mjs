import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const checker = path.join(repoRoot, "scripts/check-product-docs.mjs");
const frontmatter = path.join(repoRoot, "scripts/docs/frontmatter.mjs");
const toolingModules = path.join(repoRoot, "scripts/docs/node_modules");

function write(root, relative, content) {
  const target = path.join(root, relative);
  mkdirSync(path.dirname(target), { recursive: true });
  writeFileSync(target, content);
}

function concept(language, type = "Concept", suffix = "") {
  return `---
type: ${type}
title: Example
description: Example
language: ${language}
translation_key: concepts.example
status: stable
revision: 1
---

# Example
${suffix}
`;
}

function createFixture() {
  const root = mkdtempSync(path.join(os.tmpdir(), "iotkit-product-modes-"));
  mkdirSync(path.join(root, "scripts/docs"), { recursive: true });
  copyFileSync(checker, path.join(root, "scripts/check-product-docs.mjs"));
  copyFileSync(frontmatter, path.join(root, "scripts/docs/frontmatter.mjs"));
  write(root, "docs/product/index.md", '---\nokf_version: "0.2"\n---\n\n* [JA](ja/index.md)\n* [EN](en/index.md)\n');
  for (const language of ["ja", "en"]) {
    write(root, `docs/product/${language}/index.md`, `* [Example](concepts/example.md)\n`);
    write(root, `docs/product/${language}/concepts/example.md`, concept(language));
  }
  return root;
}

function run(mode, cwd = repoRoot) {
  const environment = { ...process.env, NODE_PATH: toolingModules };
  delete environment.OKF_BASE_REF;
  return spawnSync(process.execPath, [path.join(cwd, "scripts/check-product-docs.mjs"), `--mode=${mode}`], {
    cwd,
    encoding: "utf8",
    env: environment,
  });
}

function withFixture(callback) {
  const root = createFixture();
  try {
    callback(root);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

test("default all mode and iotkit-product pass on the repository corpus", () => {
  for (const mode of ["all", "iotkit-product", "okf-min"]) {
    const result = run(mode);
    assert.equal(result.status, 0, result.stderr || result.stdout);
    assert.match(result.stdout, new RegExp(`\\[mode=${mode}\\]`));
  }
});

test("unknown mode exits 2", () => {
  const result = spawnSync(process.execPath, [checker, "--mode=nope"], {
    cwd: repoRoot,
    encoding: "utf8",
  });
  assert.equal(result.status, 2);
});

test("okf-min tolerates producer-only type and link failures that iotkit-product rejects", () =>
  withFixture((root) => {
    for (const language of ["ja", "en"]) {
      write(root, `docs/product/${language}/concepts/example.md`, concept(language, "ExtensionType", "[Missing](missing.md)"));
    }

    const minimum = run("okf-min", root);
    const product = run("iotkit-product", root);

    assert.equal(minimum.status, 0, minimum.stderr);
    assert.notEqual(product.status, 0);
    assert.match(product.stderr, /\[iotkit-product\]/);
    assert.match(product.stderr, /unsupported type ExtensionType/);
    assert.match(product.stderr, /broken local link missing\.md/);
    assert.doesNotMatch(product.stderr, /\[okf-min\]/);
  }));

test("okf-min reports its own required type failure", () =>
  withFixture((root) => {
    write(root, "docs/product/ja/concepts/example.md", concept("ja").replace("type: Concept\n", ""));

    const result = run("okf-min", root);

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /\[okf-min\]/);
    assert.match(result.stderr, /missing required field type/);
    assert.doesNotMatch(result.stderr, /\[iotkit-product\]/);
  }));

test("all mode attributes reserved-index failures to okf-min", () =>
  withFixture((root) => {
    write(root, "docs/product/en/index.md", "---\ntype: Index\n---\n\n* [Example](concepts/example.md)\n");

    const result = run("all", root);

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /\[okf-min\]/);
    assert.match(result.stderr, /reserved index files must not have concept frontmatter/);
    assert.doesNotMatch(result.stderr, /\[iotkit-product\]/);
  }));

test("iotkit-product attributes YAML failures to the selected layer", () =>
  withFixture((root) => {
    write(root, "docs/product/ja/concepts/example.md", "---\ntype: [\n---\n");

    const result = run("iotkit-product", root);

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /\[iotkit-product\]/);
    assert.doesNotMatch(result.stderr, /\[okf-min\]/);
  }));
