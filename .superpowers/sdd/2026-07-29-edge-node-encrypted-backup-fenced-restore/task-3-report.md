# Task 3 report — authenticated Edge Node backup container

Issue: #113

Worktree: `C:\Users\watak\Documents\iotkit\.worktrees\issue-113-edge-node-backup-restore`

Base: `8f22a0a`

## Outcome

Implemented the Node-specific `IOTKNDB1` authenticated encrypted container in
`iotkit-core-recovery`:

- `encrypt_container` uses OS randomness for a bounded Argon2id/XChaCha20-Poly1305 container and hashes the exact database bytes as they are consumed for encryption.
- `authenticate_container` parses and authenticates the complete artifact while discarding plaintext.
- `decrypt_container_to_staging_file` authenticates and streams only database bytes into an anonymous OS temporary file created inside a caller-supplied staging directory; it returns the owned `DecryptedStage` handle plus manifest only after full verification, and callers provide an explicit plaintext capacity bound.
- Header and manifest parsing is closed (`deny_unknown_fields`), bounded before allocation/KDF, and uses the exact header digest, nonce, and associated-data layout from Task 3.
- Records enforce flags, sequence, plaintext/chunk lengths, authenticated terminal framing, and immediate EOF.
- Manifest database length and SHA-256 are checked while streaming; capacity is rejected before staging; failed decryptions drop the anonymous owner so the OS removes plaintext without a named path or cleanup race.
- In-place snapshot mutation/truncation during encryption is detected by the same-pass length/digest check; pathname replacement cannot switch an already-open snapshot handle.
- Passphrases, derived keys, and errors are redacted/zeroized; invalid passphrases are rejected before file/KDF work.
- Checked-in header/manifest schemas, JSON goldens, and a deterministic public binary vector are mutually conformant. Deterministic entropy is test-only in the test module; production calls `getrandom`.

## TDD evidence

### Initial RED

Ran before the container implementation:

```text
cargo test -p iotkit-core-recovery container
```

The test target compiled the existing recovery crate and failed for the
intended missing behavior: unresolved `encrypt_container_with_entropy`,
`authenticate_container`, and `decrypt_container_to_new_file`, plus missing
`RecoveryError::AuthenticationFailed` and `RecoveryError::ContainerInvalid`
(21 expected compile errors). No crypto production implementation existed at
that point.

### Round 1 GREEN

Focused final run:

```text
cargo test -p iotkit-core-recovery container
running 18 tests
17 passed, 0 failed, 1 ignored
```

The ignored test is an explicitly marked one-time public fixture generator;
the checked-in vector is exercised by the non-ignored conformance test.

Full recovery crate:

```text
cargo test -p iotkit-core-recovery
43 passed, 0 failed, 1 ignored
```

## Review-fix round

The independent review identified four Important findings: schema/Rust numeric
and identity-boundary drift, a snapshot pathname TOCTOU, missing plaintext
capacity admission, and direct plaintext publication/cleanup races. A second
test-first pass recorded the following RED evidence before implementation:

```text
cargo test -p iotkit-core-recovery container --no-run
compile failed as expected: decrypt calls lacked the new capacity argument,
publish_new_file was absent, and the schema test harness had a type mismatch
```

The fixes add checked-in JSON Schema validation of both header and manifest,
Unicode/control and integer boundary cases, integral-number deserialization
consistent with draft 2020-12 integer semantics, same-handle snapshot
encryption, a capacity check before staging, and an owner-identified RAII stage
with atomic hard-link publication. The portable replacement regression passes on
the Windows host as well as Unix targets.

Review-fix GREEN evidence:

```text
cargo test -p iotkit-core-recovery schema_and_rust_validation_agree_on_boundaries
1 passed, 0 failed

cargo test -p iotkit-core-recovery encryption_uses_the_open_snapshot_handle_after_path_replacement
1 passed, 0 failed

cargo clippy -p iotkit-core-recovery --all-targets --no-deps -- -D warnings
exit 0
```

### Review-fix round 3/5

The next review found that a named plaintext stage remained vulnerable to a
rename race under a writable parent and that a post-create encrypted-output
identity failure could leave a retry-blocking artifact. Tests were changed
first; the expected compile RED was:

```text
cargo test -p iotkit-core-recovery container --no-run
compile failed as expected: missing DecryptedStage,
decrypt_container_to_staging_file, and the injected
encrypt_snapshot_reader_with_output_init test hook
```

Plaintext publication is now removed from Task 3. The new public API accepts
the caller-supplied staging directory, creates an anonymous
`tempfile::tempfile_in` owner only after the authenticated manifest passes the
capacity check, and returns `(DecryptedStage, NodeBackupManifest)`. `File` is
private; `DecryptedStage` implements only redacted `Debug` plus controlled
`Read`/`Seek` and `rewind` access for Task 6. On authentication, digest, storage,
or unwind failure, the owner is dropped and the OS removes the file; no
plaintext pathname is created, published, unlinked, or left behind. Task 4/6
must validate the supplied directory as owner-only tmpfs before invoking this
API.

Encrypted output now uses a `NamedTempFile` cleanup owner and final
`persist_noclobber`, so a post-create initialization failure is cleaned before
the final artifact name is considered. The injected regression verifies the
temporary entry disappears and a retry can create the requested output.

Round-3 focused GREEN evidence:

```text
cargo test -p iotkit-core-recovery container
19 passed, 0 failed, 1 ignored

cargo test -p iotkit-core-recovery encrypted_output_initialization_failure_removes_temp_and_retry_succeeds
1 passed, 0 failed
```

### Review-fix round 2/5

The re-review found three remaining Important findings: noncanonical
unpadded-Base64 acceptance in the schema, a two-pass snapshot mutation gap, and
cleanup/publication races. Tests were extended before implementation. RED
evidence:

```text
cargo test -p iotkit-core-recovery schema_and_rust_validation_agree_on_boundaries
failed: schema accepted noncanonical salt_b64

cargo test -p iotkit-core-recovery container --no-run
failed as expected: encrypt_snapshot_reader and StagedOutput::cleanup were not
yet implemented
```

The final fixes use canonical 22-character Base64 (`21` alphabet symbols plus
`[AQgw]`) in both schema and Rust decoding, validate the actual checked-in
header/manifest fixtures through schema and Rust, remove the encryption
pre-pass, and compare the one-pass consumed database length/digest before
writing the terminal record. Decryption stages plaintext in a random owner-only
`0700` directory with a `0600` file, verifies file/directory identities, and
performs explicit cleanup returning a redacted `artifact_cleanup_failed` error;
`Drop` is only an unwind fallback. Windows inherited-ACL reliance is documented
for Task 4 validation.

Round-2 focused GREEN evidence:

```text
cargo test -p iotkit-core-recovery schema_and_rust_validation_agree_on_boundaries
1 passed, 0 failed

cargo test -p iotkit-core-recovery in_place_snapshot_truncation_during_encryption_fails_without_artifact
1 passed, 0 failed

cargo test -p iotkit-core-recovery staging_cleanup_failure_never_deletes_a_substituted_file
1 passed, 0 failed
```

Final full verification:

```text
cargo test -p iotkit-core-recovery container
19 passed, 0 failed, 1 ignored

cargo test -p iotkit-core-recovery
45 passed, 0 failed, 1 ignored
```

## Coverage

- Valid round trip and golden binary decrypt/re-encode byte equality.
- Every exact magic/header byte is authenticated.
- Wrong and invalid-length passphrases; wrong passphrase authentication failure.
- Edge server magic, unknown fields, algorithm dispatch, invalid base64, exact salt/nonce lengths, and every KDF/chunk bound.
- Header/manifest/chunk bounds before allocation; truncated records, malformed lengths, unknown flags, duplicate/early terminal, trailing bytes, and EOF failures.
- Modified manifest/database authentication failures and authenticated manifest length/digest mismatch.
- Existing output no-clobber and cleanup of output created before a later failure; Unix mode `0600`.
- Capacity refusal before anonymous plaintext creation; late authentication failure leaves no named plaintext and preserves unrelated staging files; success read/seek and redacted stage debug; and one-open-handle snapshot replacement regression.
- Both checked-in schemas are loaded by tests and compared with Rust parsing/validation at valid and invalid Unicode, control, integer-float, and overflow boundaries.
- Actual header/manifest fixture conformance and canonical/noncanonical 16-byte Base64 final-symbol boundaries.
- One-pass in-place snapshot truncation failure with no artifact, anonymous staging cleanup with no named file or unrelated deletion, and post-create encrypted-output initialization cleanup/retry.
- Redacted error/debug behavior and public JSON/schema/binary conformance.

## Verification

```text
cargo clippy -p iotkit-core-recovery --all-targets --no-deps -- -D warnings  # exit 0
cargo fmt --all -- --check                                           # exit 0
node scripts/check-okf-docs.mjs                                     # OK
python scripts/check-layers                                         # OK
python scripts/check-source-layout                                   # OK
git diff --check                                                     # exit 0
node scripts/battle-tested-review.mjs check                          # OK (5 entries)
node scripts/battle-tested-review.mjs select --base origin/master
```

The battle-tested selector routed BT-001 through BT-003. It also reported
unmatched recovery/schema/fixture paths; those were reviewed against the Task 3
brief and are deliberately limited to the container boundary. BT-002's
physical power-cut/storage-controller evidence remains an integrated release
gate, not something this host test can claim.

The existing unrelated `iotkit-core-ops` warning about an unused `mode` on the
Windows host remains visible in dependency builds; recovery's strict no-deps
Clippy is clean.

The round-3 commit is recorded by the final SHA in the handoff message.

## Scope and concerns

Only the recovery container, its direct model/Cargo exports, dependency lock,
schemas, fixtures, and unit tests were changed. No CLI, orchestration,
destination validation, restore state transition, release/version, or IoTKit
Edge server container work was added. Task 6 must consume the returned
`DecryptedStage` through its read/seek surface and perform fenced database
replacement; Task 3 deliberately does not publish plaintext. The public
fixture values are fixed format-vector material over a minimal sanitized
payload; they contain no deployment credential or passphrase.
