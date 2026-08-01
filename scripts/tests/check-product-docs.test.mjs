import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const projectRoot = path.resolve(import.meta.dirname, "../..");
const checkerSource = path.join(projectRoot, "scripts", "check-product-docs.mjs");

function write(repo, relative, content) {
  const target = path.join(repo, relative);
  mkdirSync(path.dirname(target), { recursive: true });
  writeFileSync(target, content);
}

function concept(language, revision, body) {
  return `---
type: Concept
title: "Example"
description: "Example concept"
language: ${language}
translation_key: concepts.example
status: stable
revision: ${revision}
---

# Example

${body}
`;
}

function runGit(repo, ...args) {
  return execFileSync("git", args, { cwd: repo, encoding: "utf8" }).trim();
}

function commit(repo, message) {
  runGit(repo, "add", "-A");
  runGit(repo, "commit", "-m", message);
  return runGit(repo, "rev-parse", "HEAD");
}

function createOldBundle() {
  const repo = mkdtempSync(path.join(os.tmpdir(), "iotkit-product-docs-"));
  runGit(repo, "init", "-b", "master");
  runGit(repo, "config", "user.name", "IoTKit Test");
  runGit(repo, "config", "user.email", "iotkit-test@example.invalid");
  runGit(repo, "config", "core.autocrlf", "false");
  mkdirSync(path.join(repo, "scripts"), { recursive: true });
  copyFileSync(checkerSource, path.join(repo, "scripts", "check-product-docs.mjs"));
  write(repo, "docs/okf/index.md", "# Old OKF bundle\n\n* [日本語](ja/index.md)\n* [English](en/index.md)\n");
  for (const language of ["ja", "en"]) {
    write(repo, `docs/okf/${language}/index.md`, "# Concepts\n\n* [Example](concepts/example.md)\n");
    write(repo, `docs/okf/${language}/concepts/example.md`, concept(language, 1, "Original body."));
  }
  const base = commit(repo, "old bundle");
  return { repo, base };
}

function migrate(repo, { revision = 1, body = "Original body." } = {}) {
  renameSync(path.join(repo, "docs", "okf"), path.join(repo, "docs", "product"));
  write(
    repo,
    "docs/product/index.md",
    '---\nokf_version: "0.2"\n---\n\n# Product docs\n\n* [日本語](ja/index.md)\n* [English](en/index.md)\n',
  );
  write(
    repo,
    "docs/okf/index.md",
    "# Moved\n\n* [Bundle](../product/index.md)\n* [English](../product/en/index.md)\n* [日本語](../product/ja/index.md)\n",
  );
  for (const language of ["ja", "en"]) {
    write(repo, `docs/product/${language}/concepts/example.md`, concept(language, revision, body));
  }
  return commit(repo, "migrate bundle");
}

function runChecker(repo, base) {
  return spawnSync(process.execPath, ["scripts/check-product-docs.mjs"], {
    cwd: repo,
    encoding: "utf8",
    env: { ...process.env, OKF_BASE_REF: base },
  });
}

function withRepo(run) {
  const fixture = createOldBundle();
  try {
    run(fixture);
  } finally {
    rmSync(fixture.repo, { recursive: true, force: true });
  }
}

test("pure docs/okf to docs/product renames do not require revision bumps", () =>
  withRepo(({ repo, base }) => {
    migrate(repo);

    const result = runChecker(repo, base);

    assert.equal(result.status, 0, result.stderr);
  }));

test("paired product-doc edits with revision bumps pass against a post-migration base", () =>
  withRepo(({ repo }) => {
    const base = migrate(repo);
    for (const language of ["ja", "en"]) {
      write(repo, `docs/product/${language}/concepts/example.md`, concept(language, 2, "Updated body."));
    }
    commit(repo, "update both translations");

    const result = runChecker(repo, base);

    assert.equal(result.status, 0, result.stderr);
  }));

test("a one-language product-doc edit is rejected", () =>
  withRepo(({ repo }) => {
    const base = migrate(repo);
    write(repo, "docs/product/ja/concepts/example.md", concept("ja", 2, "Updated Japanese body."));
    commit(repo, "update one translation");

    const result = runChecker(repo, base);

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /both translations must change together/);
  }));

test("paired product-doc edits without revision bumps are rejected", () =>
  withRepo(({ repo }) => {
    const base = migrate(repo);
    for (const language of ["ja", "en"]) {
      write(repo, `docs/product/${language}/concepts/example.md`, concept(language, 1, "Updated body."));
    }
    commit(repo, "edit without revision bump");

    const result = runChecker(repo, base);

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /revision must increase from the base version 1/);
  }));

test("delete-add migration fallback compares content with the old translation keys", () =>
  withRepo(({ repo }) => {
    const oldBody = `Old material.\n\n${"old ".repeat(200)}`;
    for (const language of ["ja", "en"]) {
      write(repo, `docs/okf/${language}/concepts/example.md`, concept(language, 1, oldBody));
    }
    commit(repo, "expand old bodies");
    const expandedBase = runGit(repo, "rev-parse", "HEAD");
    migrate(repo, { revision: 2, body: `Replacement material.\n\n${"new ".repeat(200)}` });
    const statuses = runGit(
      repo,
      "diff",
      "--name-status",
      "-M",
      `${expandedBase}...HEAD`,
      "--",
      "docs/product",
      "docs/okf",
    );
    assert.match(statuses, /^D\s+docs\/okf\/(?:en|ja)\/concepts\/example\.md$/m);
    assert.match(statuses, /^A\s+docs\/product\/(?:en|ja)\/concepts\/example\.md$/m);

    const result = runChecker(repo, expandedBase);

    assert.equal(result.status, 0, result.stderr);
  }));

test("the docs/okf compatibility stub links directly to existing product entries", () => {
  const stub = path.join(projectRoot, "docs", "okf", "index.md");
  const links = [...readFileSync(stub, "utf8").matchAll(/\[[^\]]+\]\(([^)]+)\)/g)].map((match) => match[1]);

  assert.deepEqual(links, ["../product/", "../product/en/index.md", "../product/ja/index.md", "../product/index.md"]);
  for (const href of links) {
    const target = path.resolve(path.dirname(stub), href);
    assert.equal(existsSync(target), true, `${href} must resolve from docs/okf/index.md`);
  }
});
