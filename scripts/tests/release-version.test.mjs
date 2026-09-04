import assert from "node:assert/strict";
import test from "node:test";
import {
  mkdtempSync,
  mkdirSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  REQUIRED_COMPATIBILITY_DOMAINS,
  REQUIRED_STORAGE_SCHEMAS,
  extractChangelogReleases,
  extractChangelogVersions,
  extractEnglishReadmeRelease,
  extractEnglishReadmeVersion,
  extractJapaneseReadmeRelease,
  extractJapaneseReadmeVersion,
  extractWorkspaceVersion,
  packageInheritsWorkspaceVersion,
  validateCompatibilityManifest,
  validateReleaseState,
} from "../check-release-version.mjs";

function writeFixtureFile(root, relative) {
  const path = join(root, relative);
  mkdirSync(join(path, ".."), { recursive: true });
  writeFileSync(path, "fixture\n");
}

function compatibleManifest() {
  return {
    schema_version: 1,
    domains: REQUIRED_COMPATIBILITY_DOMAINS.map((id) => ({
      id,
      authority: ["docs/policy.md"],
      types: ["types/reference.rs"],
      schemas: ["schemas/contract.json"],
      fixtures: ["fixtures/case.json"],
      tests: ["tests/conformance.rs"],
    })),
    storage: REQUIRED_STORAGE_SCHEMAS.map((id) => ({
      id,
      schema_version: 2,
      authority: ["docs/policy.md"],
      schema: storageMigrationPaths(id),
      tests: ["tests/conformance.rs"],
    })),
  };
}

function storageMigrationPaths(id) {
  return id === "edge-node-sqlite"
    ? ["migrations/edge-node-a", "migrations/edge-node-b"]
    : [`migrations/${id}`];
}

function withFixtureRepository(run) {
  const root = mkdtempSync(join(tmpdir(), "iotkit-release-version-"));
  try {
    for (const path of [
      "docs/policy.md",
      "types/reference.rs",
      "schemas/contract.json",
      "fixtures/case.json",
      "tests/conformance.rs",
    ]) {
      writeFixtureFile(root, path);
    }
    for (const id of REQUIRED_STORAGE_SCHEMAS) {
      for (const path of storageMigrationPaths(id)) {
        writeFixtureFile(root, `${path}/0001_initial.sql`);
        writeFixtureFile(root, `${path}/0002_current.sql`);
      }
    }
    run(root);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

test("extracts the workspace package version", () => {
  assert.equal(
    extractWorkspaceVersion(
      `[workspace]\nmembers = []\n\n[workspace.package]\nversion = "0.1.0"\n`,
    ),
    "0.1.0",
  );
});

test("accepts one inherited 0.1.0 product version", () => {
  assert.deepEqual(
    validateReleaseState({
      version: "0.1.0",
      packages: [
        {
          name: "iotkit-edge",
          version: "0.1.0",
          inheritsVersion: true,
        },
      ],
      repository: "https://github.com/w-pinkietech/iotkit",
      tag: "v0.1.0",
    }),
    [],
  );
});

test("reports package, repository, and tag drift together", () => {
  const problems = validateReleaseState({
    version: "0.1.0",
    packages: [
      {
        name: "iotkit-edge-node",
        version: "0.2.0",
        inheritsVersion: false,
      },
    ],
    repository: "https://github.com/w-pinkietech/iotkit-next",
    tag: "0.1.0",
  });

  assert.equal(problems.length, 4);
});

test("extracts the bilingual README and changelog version markers", () => {
  assert.equal(
    extractEnglishReadmeVersion(
      "> **Current product version: 0.1.0 (pre-1.0).**\n",
    ),
    "0.1.0",
  );
  assert.equal(
    extractJapaneseReadmeVersion(
      "> **現在の製品バージョン: 0.1.0（pre-1.0）。**\n",
    ),
    "0.1.0",
  );
  assert.deepEqual(
    extractEnglishReadmeRelease(
      "> **Current product version: 1.0.0 (stable).**\n",
    ),
    { version: "1.0.0", marker: "stable" },
  );
  assert.deepEqual(
    extractJapaneseReadmeRelease(
      "> **現在の製品バージョン: 1.0.0（stable）。**\n",
    ),
    { version: "1.0.0", marker: "stable" },
  );
  assert.deepEqual(
    extractChangelogVersions("## [Unreleased]\n\n## [0.1.0] - 2026-07-27\n"),
    ["0.1.0"],
  );
});

test("requires README lifecycle markers that match the product major", () => {
  assert.deepEqual(
    validateReleaseState({
      version: "0.4.0",
      packages: [],
      repository: "https://github.com/w-pinkietech/iotkit",
      readmeVersion: "0.4.0",
      readmeMarker: "stable",
      readmeJaVersion: "0.4.0",
      readmeJaMarker: "stable",
    }),
    [
      "README product lifecycle marker is stable, expected pre-1.0 for 0.4.0",
      "README.ja product lifecycle marker is stable, expected pre-1.0 for 0.4.0",
    ],
  );
  assert.deepEqual(
    validateReleaseState({
      version: "1.0.0",
      packages: [],
      repository: "https://github.com/w-pinkietech/iotkit",
      readmeVersion: "1.0.0",
      readmeMarker: "pre-1.0",
      readmeJaVersion: "1.0.0",
      readmeJaMarker: "pre-1.0",
    }),
    [
      "README product lifecycle marker is pre-1.0, expected stable for 1.0.0",
      "README.ja product lifecycle marker is pre-1.0, expected stable for 1.0.0",
    ],
  );
});

test("rejects stable README markers with pre-1.0 status text", () => {
  assert.deepEqual(
    validateReleaseState({
      version: "1.0.0",
      packages: [],
      repository: "https://github.com/w-pinkietech/iotkit",
      readmeVersion: "1.0.0",
      readmeMarker: "stable",
      readmeStatus: "IoTKit is available as a stable source release.",
      readmeJaVersion: "1.0.0",
      readmeJaMarker: "stable",
      readmeJaStatus: "IoTKitは安定source releaseとして公開しています。",
    }),
    [],
  );

  const problems = validateReleaseState({
    version: "1.0.0",
    packages: [],
    repository: "https://github.com/w-pinkietech/iotkit",
    readmeVersion: "1.0.0",
    readmeMarker: "stable",
    readmeStatus:
      "IoTKit is available as an early source release. This pre-1.0 status remains.",
    readmeJaVersion: "1.0.0",
    readmeJaMarker: "stable",
    readmeJaStatus:
      "IoTKitは早期source releaseとして公開しています。このpre-1.0の状態を変更しません。",
  });

  assert.deepEqual(problems, [
    "README status block must describe a stable source release for 1.0.0",
    "README status block still contains pre-1.0 wording for 1.0.0",
    "README.ja status block must describe a stable source release for 1.0.0",
    "README.ja status block still contains pre-1.0 wording for 1.0.0",
  ]);
});

test("rejects malformed Cargo versions and README version drift", () => {
  for (const version of ["0.1", "v0.1.0"]) {
    assert.match(
      validateReleaseState({
        version,
        packages: [],
        repository: "https://github.com/w-pinkietech/iotkit",
      })[0],
      /MAJOR\.MINOR\.PATCH SemVer/,
    );
  }

  assert.deepEqual(
    validateReleaseState({
      version: "0.1.0",
      packages: [],
      repository: "https://github.com/w-pinkietech/iotkit",
      readmeVersion: "0.2.0",
      readmeJaVersion: "0.1.0",
      changelogVersions: ["0.1.0"],
    }),
    ["README product version is 0.2.0, expected 0.1.0"],
  );
});

test("accepts workspace version inheritance only from the package table", () => {
  assert.equal(
    packageInheritsWorkspaceVersion(
      `[package]\nname = "iotkit-edge"\nversion.workspace = true\n`,
    ),
    true,
  );
  assert.equal(
    packageInheritsWorkspaceVersion(
      `[package]\nname = "iotkit-edge"\nversion = "0.1.0"\n\n[package.metadata.release]\nversion.workspace = true\n`,
    ),
    false,
  );
});

test("reports malformed and duplicate changelog release headings", () => {
  const changelogReleases = extractChangelogReleases(
    [
      "## [Unreleased]",
      "",
      "## [0.1.0] - 2026-07-27",
      "",
      "## [0.1.0] - July 28",
      "",
      "## [v0.2.0] - 2026-08-01",
    ].join("\n"),
  );
  const problems = validateReleaseState({
    version: "0.1.0",
    packages: [],
    repository: "https://github.com/w-pinkietech/iotkit",
    changelogReleases,
  });

  assert.deepEqual(problems, [
    "CHANGELOG.md has duplicate release heading for 0.1.0",
    "CHANGELOG.md release date is invalid for 0.1.0: July 28",
    "CHANGELOG.md version is not SemVer: v0.2.0",
  ]);
});

test("requires a closed, complete compatibility manifest with local evidence", () => {
  withFixtureRepository((root) => {
    const manifest = compatibleManifest();
    assert.deepEqual(validateCompatibilityManifest(manifest, root), []);

    manifest.unexpected = true;
    manifest.domains[0].authority = ["../outside.md"];
    manifest.domains[1].id = manifest.domains[0].id;
    manifest.storage.pop();

    const problems = validateCompatibilityManifest(manifest, root);
    assert.ok(problems.some((problem) => problem.includes("unexpected key unexpected")));
    assert.ok(problems.some((problem) => problem.includes("must be a safe repository-relative path")));
    assert.ok(problems.some((problem) => problem.includes("duplicate id")));
    assert.ok(problems.some((problem) => problem.includes("missing required storage schema")));
  });
});

test("requires non-empty evidence and rejects malformed nested entries", () => {
  withFixtureRepository((root) => {
    const manifest = compatibleManifest();
    manifest.domains[0].authority = [];
    manifest.domains[0].fixtures = [];
    manifest.domains[1].nested_unknown = true;
    manifest.domains[1].types = "types/reference.rs";
    manifest.domains[2].schemas = [];
    manifest.domains[2].tests = [];
    manifest.storage[0].authority = [];
    manifest.storage[0].schema = [];
    manifest.storage[0].tests = [];
    manifest.storage.push(null);

    const problems = validateCompatibilityManifest(manifest, root);
    assert.ok(problems.some((problem) => problem.includes("domains[0].authority must not be empty")));
    assert.ok(problems.some((problem) => problem.includes("domains[1] has unexpected key nested_unknown")));
    assert.ok(problems.some((problem) => problem.includes("domains[1].types must be an array")));
    assert.ok(problems.some((problem) => problem.includes("domains[2].schemas must not be empty")));
    assert.ok(problems.some((problem) => problem.includes("domains[2].tests must not be empty")));
    assert.ok(problems.some((problem) => problem.includes("storage[0].authority must not be empty")));
    assert.ok(problems.some((problem) => problem.includes("storage[0].schema must not be empty")));
    assert.ok(problems.some((problem) => problem.includes("storage[0].tests must not be empty")));
    assert.ok(problems.some((problem) => problem.includes("storage[1] must be an object")));
    assert.ok(!problems.some((problem) => problem.includes("domains[0].fixtures must not be empty")));
  });
});

test("binds each storage schema version to its listed migrations", () => {
  withFixtureRepository((root) => {
    const manifest = compatibleManifest();
    writeFixtureFile(root, "migrations/edge-node-b/0003_new.sql");

    const problems = validateCompatibilityManifest(manifest, root);
    assert.ok(problems.some((problem) => problem.includes("storage[0].schema_version 2 does not match migration version 3")));
  });
});

test("rejects evidence paths through intermediate symlinks", () => {
  const outside = mkdtempSync(join(tmpdir(), "iotkit-release-version-outside-"));
  try {
    writeFileSync(join(outside, "escape.rs"), "fixture\n");
    withFixtureRepository((root) => {
      const manifest = compatibleManifest();
      mkdirSync(join(root, "linked"));
      symlinkSync(outside, join(root, "linked", "outside"), "dir");
      manifest.domains[0].types = ["linked/outside/escape.rs"];

      const problems = validateCompatibilityManifest(manifest, root);
      assert.ok(problems.some((problem) => problem.includes("resolves outside the repository root")));
    });
  } finally {
    rmSync(outside, { recursive: true, force: true });
  }
});

test("rejects numeric migration symlinks", () => {
  const outside = mkdtempSync(join(tmpdir(), "iotkit-release-version-migration-outside-"));
  try {
    writeFileSync(join(outside, "0003_escape.sql"), "fixture\n");
    withFixtureRepository((root) => {
      const manifest = compatibleManifest();
      symlinkSync(
        join(outside, "0003_escape.sql"),
        join(root, "migrations", "edge-node-a", "0003_escape.sql"),
      );

      const problems = validateCompatibilityManifest(manifest, root);
      assert.ok(problems.some((problem) => problem.includes("schema must not contain symbolic-link migration 0003_escape.sql")));
    });
  } finally {
    rmSync(outside, { recursive: true, force: true });
  }
});
