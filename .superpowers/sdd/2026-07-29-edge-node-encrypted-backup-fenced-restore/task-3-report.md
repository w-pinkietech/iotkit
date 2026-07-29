# Task 3 report — authenticated Edge Node backup container

Issue: #113

Worktree: `C:\Users\watak\Documents\iotkit\.worktrees\issue-113-edge-node-backup-restore`

Base: `8f22a0a`

## Outcome

Implemented the Node-specific `IOTKNDB1` authenticated encrypted container in
`iotkit-core-recovery`:

- `encrypt_container` uses OS randomness for a bounded Argon2id/XChaCha20-Poly1305 container.
- `authenticate_container` parses and authenticates the complete artifact while discarding plaintext.
- `decrypt_container_to_new_file` authenticates and streams only database bytes into a private staging file, then publishes with atomic no-clobber semantics after full verification; callers provide an explicit plaintext capacity bound.
- Header and manifest parsing is closed (`deny_unknown_fields`), bounded before allocation/KDF, and uses the exact header digest, nonce, and associated-data layout from Task 3.
- Records enforce flags, sequence, plaintext/chunk lengths, authenticated terminal framing, and immediate EOF.
- Manifest database length and SHA-256 are checked while streaming; capacity is rejected before staging; failed decryptions remove only their owner-identified staging file and never replace an existing or concurrently substituted path.
- Encryption verifies and rewinds one open snapshot handle, so pathname replacement cannot switch the bytes being encrypted.
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

### GREEN

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

## Coverage

- Valid round trip and golden binary decrypt/re-encode byte equality.
- Every exact magic/header byte is authenticated.
- Wrong and invalid-length passphrases; wrong passphrase authentication failure.
- Edge server magic, unknown fields, algorithm dispatch, invalid base64, exact salt/nonce lengths, and every KDF/chunk bound.
- Header/manifest/chunk bounds before allocation; truncated records, malformed lengths, unknown flags, duplicate/early terminal, trailing bytes, and EOF failures.
- Modified manifest/database authentication failures and authenticated manifest length/digest mismatch.
- Existing output no-clobber and cleanup of output created before a later failure; Unix mode `0600`.
- Capacity refusal before plaintext creation; late authentication failure staging cleanup; destination replacement/no-delete regression; and one-open-handle snapshot replacement regression.
- Both checked-in schemas are loaded by tests and compared with Rust parsing/validation at valid and invalid Unicode, control, integer-float, and overflow boundaries.
- Redacted error/debug behavior and public JSON/schema/binary conformance.

## Verification

```text
cargo clippy -p iotkit-core-recovery --all-targets --no-deps -- -D warnings  # exit 0
cargo fmt --all -- --check                                           # exit 0
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

## Scope and concerns

Only the recovery container, its direct model/Cargo exports, schemas, fixtures,
and unit tests were changed. No CLI, orchestration, destination validation,
restore state transition, release/version, or IoTKit Edge server container work
was added. The public fixture values are fixed format-vector material over a
minimal sanitized payload; they contain no deployment credential or passphrase.
