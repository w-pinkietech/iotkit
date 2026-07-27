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
node scripts/check-release-version.mjs --tag v0.1.0
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
git fetch origin master --tags
test "$(git rev-parse HEAD)" = "$(git rev-parse origin/master)"
git tag -a v0.1.0 -m "IoTKit v0.1.0"
git push origin refs/tags/v0.1.0
gh release create v0.1.0 \
  --repo w-pinkietech/iotkit \
  --verify-tag \
  --title "IoTKit v0.1.0" \
  --generate-notes \
  --notes "Early pre-1.0 source release. Product and public contracts may change during the 0.x series."
```

The normal release procedure never force-moves or deletes a tag. If a tag or
remote commit differs, stop and investigate instead of overwriting it.

This is a source-only release. GitHub-generated source archives are the only
assets; do not attach binaries, container images, OS images, signatures,
checksums, SBOMs, or update artifacts.
