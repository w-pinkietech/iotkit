# Source Release Versioning Design

Date: 2026-07-27
Issue: [#102](https://github.com/w-pinkietech/iotkit/issues/102)
Status: approved

## 1. Purpose

IoTKit is public, but it has no Git tag or GitHub Release and does not expose one
consistent product version. The Rust crates currently declare `0.1.0`, while the
README files call the repository a v1 release candidate.

This change establishes one pre-1.0 product version for the whole monorepo,
makes that version visible to users and operators, and documents a repeatable
source-only GitHub Release procedure.

## 2. Scope

The first product release is `0.1.0`, represented by the annotated Git tag
`v0.1.0`.

The release contains only the source archives GitHub generates for a tag. It
does not publish binaries, container images, OS images, checksums, signatures,
or an SBOM. Those distribution concerns remain in #95. Public contract
stability, compatibility periods, and v1 policy remain in #96.

Product versioning is separate from versioned MQTT, API, disk, snapshot,
adapter, configuration, and OKF formats. A product release of `0.1.0` may
continue to implement a wire contract named `v1`; neither version implies the
other.

## 3. Version authority

The root `Cargo.toml` owns the product version:

```toml
[workspace.package]
version = "0.1.0"
license = "Apache-2.0"
repository = "https://github.com/w-pinkietech/iotkit"
```

Every Cargo workspace member inherits it with:

```toml
[package]
version.workspace = true
```

Non-workspace layer fixtures under `scripts/layer-fixtures/` remain test data
and keep their explicit `0.0.0` versions.

There is no separate `VERSION` file. Avoiding a second authority prevents
Cargo metadata and release metadata from drifting.

## 4. Pre-1.0 policy

IoTKit uses semantic version syntax with the following pre-1.0 interpretation:

- `0.MINOR.0` introduces features or may intentionally change compatibility.
- `0.MINOR.PATCH` fixes behavior without intentionally changing compatibility.
- `1.0.0` is reserved for the compatibility decision tracked by #96.

Every release is tagged `vX.Y.Z`. The tag prefix is presentation and Git
convention; the Cargo version itself has no `v` prefix.

All product components advance together. IoTKit Edge, IoTKit Edge Node,
operator CLIs, adapters, testkits, and internal crates do not receive
independent product versions during the 0.x series.

## 5. User-visible version

The same Cargo workspace version appears in these surfaces:

1. `README.md` and `README.ja.md` identify the current release as `0.1.0`,
   describe it as pre-1.0, and link to the GitHub Releases page.
2. `iotkit-edge --version` prints `iotkit-edge 0.1.0`.
3. `iotkit-edge-node --version` prints `iotkit-edge-node 0.1.0` without
   starting the service or opening storage.
4. `iotkit-edge-nodectl --version` prints `iotkit-edge-nodectl 0.1.0` without
   opening a database.
5. The authenticated Console system page shows `IoTKit Edge 0.1.0`.
6. The existing Edge Node HTTP status response continues to return
   `CARGO_PKG_VERSION`, which resolves to the same workspace version.

Build commit identifiers are not added in this change. The source tag is the
release identity, while development builds report their Cargo product version.

## 6. Changelog

The repository gains a single top-level `CHANGELOG.md` for product releases.
It begins with an `Unreleased` section followed by `0.1.0`. Entries describe
user-visible behavior and operational changes; they do not enumerate every
internal commit.

Each later version PR moves completed entries from `Unreleased` into a dated
version section and updates the workspace version. Contract changes identify
their contract or schema version explicitly so readers do not confuse them
with the product version.

## 7. Source release procedure

`RELEASING.md` documents this maintainer-controlled sequence:

1. Prepare a version PR that updates the workspace version, `Cargo.lock`, both
   README files, and `CHANGELOG.md`.
2. Run the version consistency check and the repository verification required
   by the changed paths.
3. Merge the reviewed PR to `master`.
4. Confirm the local checkout is clean, `master` matches `origin/master`, and
   no tag or GitHub Release already exists for the version.
5. Create an annotated `vX.Y.Z` tag at that exact `master` commit.
6. Push only that tag.
7. Create a normal GitHub Release from the tag with release notes that identify
   it as an early pre-1.0 release.
8. Verify that the release page exposes GitHub's generated source archives and
   that the README release link resolves.

Tag creation, tag push, and GitHub Release publication remain explicit release
actions. Implementing this design does not perform them. The first `v0.1.0`
release occurs only after the implementation PR is merged and the maintainer
explicitly approves publication.

No GitHub Actions release workflow is added. A source-only release does not
need artifact build automation, and keeping publication manual preserves the
repository's release authority boundary.

## 8. Consistency check

A repository script checks release metadata without mutating files:

- the root workspace version is valid `MAJOR.MINOR.PATCH` SemVer;
- all Cargo workspace packages resolve to the root version;
- the English and Japanese README current-version markers match it;
- `CHANGELOG.md` has a section for it;
- the repository URL is `https://github.com/w-pinkietech/iotkit`;
- when a tag is supplied in CI or by a maintainer, it is exactly
  `v${workspaceVersion}`.

The script uses `cargo metadata --no-deps` to resolve inherited package
versions instead of implementing a partial TOML parser. CI runs the check on
every pull request. A mismatch fails before release preparation can continue.

## 9. Failure handling

Release preparation stops without changing remote state when:

- the version is not valid SemVer;
- a workspace package resolves to another version;
- README or changelog metadata does not match;
- the working tree is dirty;
- local `master` differs from `origin/master`;
- the intended tag is not `v${workspaceVersion}`;
- the tag or GitHub Release already exists.

The release runbook never force-moves or deletes a tag. Correcting a published
release requires a separate maintainer decision; the normal procedure creates
a new version.

CLI `--version` paths must not initialize storage, read credentials, contact a
Broker, or start a listener. The Console obtains its version from compile-time
Cargo metadata and does not read a mutable file at runtime.

## 10. Verification

Focused verification covers:

- the version consistency script with matching and mismatching fixtures;
- `--version` output for all three shipped binaries;
- the Edge Node status response version;
- the Console system-page version for authenticated viewers;
- README and changelog markers through the consistency script;
- existing documentation and source-layout checks for all changed files.

Rust product behavior outside version reporting is unchanged. The full
repository verification remains required before the draft PR is handed off
because the workspace manifest and lockfile affect every crate.

## 11. Delivery

This work uses branch `agent/issue-102-source-release-versioning` and closes
#102 through a draft pull request. The implementation PR prepares the
repository for `v0.1.0` but does not publish the tag or GitHub Release.

After human review and merge:

1. request explicit publication approval;
2. follow `RELEASING.md` at the merged commit;
3. verify the public `v0.1.0` release;
4. resume Issue #101 in its own branch and worktree.
