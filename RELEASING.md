# Releasing IoTKit

IoTKit uses one product version for the complete Cargo workspace. The authority
is `workspace.package.version` in the root `Cargo.toml`; API, MQTT, disk,
snapshot, adapter, configuration, and OKF format versions remain independent
contract identifiers.

During pre-1.0 development:

- `0.MINOR.0` may add features or intentionally change compatibility.
- `0.MINOR.PATCH` is for compatible fixes and does not intentionally change
  compatibility.

## Prepare a source release

1. Choose `X.Y.Z` according to the policy above and update the root workspace
   version. Every workspace crate must continue to use
   `version.workspace = true`.
2. Update the current-version marker in `README.md` and `README.ja.md`.
3. Move the release notes from `Unreleased` into a dated
   `## [X.Y.Z] - YYYY-MM-DD` section in `CHANGELOG.md`.
4. Run `cargo metadata --no-deps --format-version 1` if Cargo needs to refresh
   workspace metadata, and inspect any `Cargo.lock` change.
5. Open and merge the release-preparation pull request before publishing.

For the initial release, verify the exact version and full repository gates:

```bash
set -euo pipefail
version="$(node --input-type=module -e \
  'import { readFileSync } from "node:fs"; import { extractWorkspaceVersion } from "./scripts/check-release-version.mjs"; process.stdout.write(extractWorkspaceVersion(readFileSync("Cargo.toml", "utf8")));')"
tag="v${version}"
node scripts/check-release-version.mjs --tag "$tag"
scripts/verify.sh
git status --short --branch
```

The checker refuses a tag that does not exactly equal `v` plus the workspace
version. The worktree must be clean before continuing.

## Publish from the merged commit

Tag creation, tag push, and `gh release create` each require explicit maintainer
approval. After approval, fetch the current remote state and prove that the
checked-out commit is exactly the merged `master` commit:

```bash
set -euo pipefail
version="$(node --input-type=module -e \
  'import { readFileSync } from "node:fs"; import { extractWorkspaceVersion } from "./scripts/check-release-version.mjs"; process.stdout.write(extractWorkspaceVersion(readFileSync("Cargo.toml", "utf8")));')"
tag="v${version}"
node scripts/check-release-version.mjs --tag "$tag"
git fetch origin master --tags
test -z "$(git status --porcelain)"
test "$(git rev-parse HEAD)" = "$(git rev-parse origin/master)"
if git rev-parse --verify --quiet "refs/tags/$tag"; then
  echo "tag already exists: $tag" >&2
  exit 1
fi
if gh release view "$tag" --repo w-pinkietech/iotkit >/dev/null 2>&1; then
  echo "GitHub Release already exists: $tag" >&2
  exit 1
fi
git tag -a "$tag" -m "IoTKit $tag"
git push origin "refs/tags/$tag"
gh release create "$tag" \
  --repo w-pinkietech/iotkit \
  --verify-tag \
  --title "IoTKit $tag" \
  --generate-notes \
  --notes "Early pre-1.0 source release. Product and public contracts may change during the 0.x series."
gh release view "$tag" \
  --repo w-pinkietech/iotkit \
  --json isDraft,tagName,url
```

The normal release procedure never force-moves or deletes a tag. If a tag or
remote commit differs, stop and investigate instead of overwriting it.

This is a source-only release. GitHub-generated source archives are the only
assets; do not attach binaries, container images, OS images, signatures,
checksums, SBOMs, or update artifacts.
