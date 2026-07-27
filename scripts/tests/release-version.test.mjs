import assert from "node:assert/strict";
import test from "node:test";
import {
  extractWorkspaceVersion,
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

test("reports package, document, repository, and tag drift together", () => {
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
