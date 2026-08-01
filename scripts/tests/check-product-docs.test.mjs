import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
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
  runGit(repo, "config", "commit.gpgsign", "false");
  runGit(repo, "config", "tag.gpgsign", "false");
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

function migrate(repo, { revision = 1, body = "Original body.", forwardingStubs = true } = {}) {
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
    if (forwardingStubs) {
      write(
        repo,
        `docs/okf/${language}/index.md`,
        `# Moved\n\n* [Replacement](../../product/${language}/index.md)\n`,
      );
      write(
        repo,
        `docs/okf/${language}/concepts/example.md`,
        `# Moved\n\n* [Replacement](../../../product/${language}/concepts/example.md)\n`,
      );
    }
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
    assert.match(result.stdout, /Product docs \(IoTKit producer profile; OKF v0\.2 packaging\) validation passed/);
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

test("forwarding-stub-only edits after migration do not require product revision bumps", () =>
  withRepo(({ repo }) => {
    const base = migrate(repo);
    write(
      repo,
      "docs/okf/en/concepts/example.md",
      "# Moved permanently\n\n* [Replacement](../../../product/en/concepts/example.md)\n",
    );
    commit(repo, "clarify forwarding stub");

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

test("malformed link escapes are reported without aborting the checker", () =>
  withRepo(({ repo }) => {
    const base = migrate(repo);
    write(
      repo,
      "docs/product/en/index.md",
      "# Concepts\n\n* [Example](concepts/example.md)\n* [Malformed](bad%zz.md)\n",
    );
    commit(repo, "add malformed link");

    const result = runChecker(repo, base);

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /Product docs \(IoTKit producer profile\) validation failed/);
    assert.match(result.stderr, /local link is not a valid URI reference: bad%zz\.md/);
    assert.doesNotMatch(result.stderr, /URIError|decodeURIComponent/);
  }));

test("a missing product bundle root index is reported without an ENOENT stack", () =>
  withRepo(({ repo }) => {
    const base = migrate(repo);
    rmSync(path.join(repo, "docs", "product", "index.md"));
    commit(repo, "remove product root index");

    const result = runChecker(repo, base);

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /Product docs \(IoTKit producer profile\) validation failed/);
    assert.match(result.stderr, /docs[\\/]product[\\/]index\.md: does not exist/);
    assert.doesNotMatch(result.stderr, /ENOENT|readFileSync/);
  }));

test("a missing product bundle uses the IoTKit producer-profile failure banner", () =>
  withRepo(({ repo }) => {
    const base = migrate(repo);
    rmSync(path.join(repo, "docs", "product"), { recursive: true });
    commit(repo, "remove product bundle");

    const result = runChecker(repo, base);

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /Product docs \(IoTKit producer profile\) validation failed/);
    assert.match(result.stderr, /docs[\\/]product: does not exist/);
  }));

test("delete-add migration fallback compares content with the old translation keys", () =>
  withRepo(({ repo }) => {
    const oldBody = `Old material.\n\n${"old ".repeat(200)}`;
    for (const language of ["ja", "en"]) {
      write(repo, `docs/okf/${language}/concepts/example.md`, concept(language, 1, oldBody));
    }
    commit(repo, "expand old bodies");
    const expandedBase = runGit(repo, "rev-parse", "HEAD");
    migrate(repo, {
      revision: 2,
      body: `Replacement material.\n\n${"new ".repeat(200)}`,
      forwardingStubs: false,
    });
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

  assert.deepEqual(
    [...links].sort(),
    ["../product/", "../product/en/index.md", "../product/ja/index.md", "../product/index.md"].sort(),
  );
  for (const href of links) {
    const target = path.resolve(path.dirname(stub), href);
    assert.equal(existsSync(target), true, `${href} must resolve from docs/okf/index.md`);
  }
});

function markdownFiles(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const target = path.join(directory, entry.name);
    if (entry.isDirectory()) return markdownFiles(target);
    return entry.isFile() && entry.name.endsWith(".md") ? [target] : [];
  });
}

test("former deep docs/okf paths forward one hop to every product document", () => {
  const productRoot = path.join(projectRoot, "docs", "product");
  const oldRoot = path.join(projectRoot, "docs", "okf");
  for (const productFile of markdownFiles(productRoot)) {
    const relative = path.relative(productRoot, productFile);
    if (relative === "index.md") continue;
    const stub = path.join(oldRoot, relative);
    assert.equal(existsSync(stub), true, `${path.relative(projectRoot, stub)} must remain as a forwarding stub`);
    const expectedHref = path.relative(path.dirname(stub), productFile).split(path.sep).join("/");
    const links = [...readFileSync(stub, "utf8").matchAll(/\[[^\]]+\]\(([^)]+)\)/g)].map((match) => match[1]);
    assert.deepEqual(links, [expectedHref], `${path.relative(projectRoot, stub)} must link directly to its replacement`);
  }
});
