import assert from "node:assert/strict";
import test from "node:test";
import {
  extractChangelogReleases,
  extractChangelogVersions,
  extractEnglishReadmeVersion,
  extractJapaneseReadmeVersion,
  extractWorkspaceVersion,
  packageInheritsWorkspaceVersion,
  validateReleaseState,
} from "../check-release-version.mjs";

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
    extractChangelogVersions("## [Unreleased]\n\n## [0.1.0] - 2026-07-27\n"),
    ["0.1.0"],
  );
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
