# Releasing IoTKit

IoTKit uses one product version for the complete Cargo workspace. The authority
is `workspace.package.version` in the root `Cargo.toml`; API, MQTT, disk,
snapshot, adapter, configuration, and OKF format versions remain independent
contract identifiers.

During pre-1.0 development:

- `0.MINOR.0` may add features or intentionally change compatibility.
- `0.MINOR.PATCH` is for compatible fixes and does not intentionally change
  compatibility.

Starting with product 1.0.0, follow the product-1.x promise in
[`docs/product/en/contracts/compatibility-policy-v1.md`](docs/product/en/contracts/compatibility-policy-v1.md):
minor releases may add backward-compatible public behavior and compatible
fixes; patch releases contain compatible fixes and do not intentionally add
features. Both preserve supported public contract majors, while version domains
remain independent. Before releasing any change to a public contract or storage
schema, update its release evidence in
`testdata/compatibility/v1/release-manifest.json` together with the paired
product documents, types/schemas, fixtures, and tests. The source tag/archive,
not a moving branch URL, is the release artifact for that evidence.

## Prepare a source release

1. Choose `X.Y.Z` according to the policy above and update the root workspace
   version. Every workspace crate must continue to use
   `version.workspace = true`.
2. Update the complete current-version status block in `README.md` and
   `README.ja.md`: `0.x` uses `(pre-1.0)` / `（pre-1.0）` with the early-source
   wording. For `1.x`, use `(stable)` / `（stable）` and replace that wording
   with `IoTKit is available as a stable source release.` /
   `IoTKitは安定source releaseとして公開しています。`, followed by the applicable
   v1 compatibility-policy link. A later product major uses its own applicable
   policy. Remove the pre-1.0 status statement; the release checker rejects a
   stable marker that retains it.
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
scripts/verify.sh --workspace
scripts/test-journey.sh
git status --short --branch
```

The checker refuses a tag that does not exactly equal `v` plus the workspace
version. The worktree must be clean before continuing. The journey is the
release integration suite; real-device evidence (journey stage L4) remains
outside the release default.

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
case "$version" in
  0.*) release_notes="Early pre-1.0 source release. Product and public contracts may change during the 0.x series." ;;
  1.*) release_notes="Stable source release. See the v1 compatibility policy for supported public contracts." ;;
  *) release_notes="Stable source release." ;;
esac
git tag -a "$tag" -m "IoTKit $tag"
git push origin "refs/tags/$tag"
gh release create "$tag" \
  --repo w-pinkietech/iotkit \
  --verify-tag \
  --title "IoTKit $tag" \
  --generate-notes \
  --notes "$release_notes"
gh release view "$tag" \
  --repo w-pinkietech/iotkit \
  --json isDraft,tagName,url
```

The normal release procedure never force-moves or deletes a tag. If a tag or
remote commit differs, stop and investigate instead of overwriting it.

This is a source-only release. GitHub-generated source archives are the only
assets; do not attach binaries, container images, OS images, signatures,
checksums, SBOMs, or update artifacts.
