# Task 3 report — authenticated Edge Node backup container

Issue: #113

Worktree: `C:\Users\watak\Documents\iotkit\.worktrees\issue-113-edge-node-backup-restore`

Base: `8f22a0a`

## Outcome

Implemented the Node-specific `IOTKNDB1` authenticated encrypted container in
`iotkit-core-recovery`:

- `encrypt_container` uses OS randomness for a bounded Argon2id/XChaCha20-Poly1305 container.
- `authenticate_container` parses and authenticates the complete artifact while discarding plaintext.
- `decrypt_container_to_new_file` authenticates and streams only database bytes into a new owner-only file.
- Header and manifest parsing is closed (`deny_unknown_fields`), bounded before allocation/KDF, and uses the exact header digest, nonce, and associated-data layout from Task 3.
- Records enforce flags, sequence, plaintext/chunk lengths, authenticated terminal framing, and immediate EOF.
- Manifest database length and SHA-256 are checked while streaming; failed decryptions remove only the output created by that call and never replace an existing path.
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
running 14 tests
13 passed, 0 failed, 1 ignored
```

The ignored test is an explicitly marked one-time public fixture generator;
the checked-in vector is exercised by the non-ignored conformance test.

Full recovery crate:

```text
cargo test -p iotkit-core-recovery
39 passed, 0 failed, 1 ignored
```

## Coverage

- Valid round trip and golden binary decrypt/re-encode byte equality.
- Every exact magic/header byte is authenticated.
- Wrong and invalid-length passphrases; wrong passphrase authentication failure.
- Edge server magic, unknown fields, algorithm dispatch, invalid base64, exact salt/nonce lengths, and every KDF/chunk bound.
- Header/manifest/chunk bounds before allocation; truncated records, malformed lengths, unknown flags, duplicate/early terminal, trailing bytes, and EOF failures.
- Modified manifest/database authentication failures and authenticated manifest length/digest mismatch.
- Existing output no-clobber and cleanup of output created before a later failure; Unix mode `0600`.
- Redacted error/debug behavior and public JSON/schema/binary conformance.

## Verification

```text
cargo clippy -p iotkit-core-recovery --all-targets --no-deps -- -D warnings  # exit 0
cargo fmt --all -- --check                                           # exit 0
python scripts/check-layers                                         # OK
python scripts/check-source-layout                                   # OK
git diff --check                                                     # exit 0
node scripts/battle-tested-review.mjs check                          # OK (5 entries)
node scripts/battle-tested-review.mjs select --base origin/master \
  --concern edge-node-replacement --concern custody --concern restore \
  --concern power-loss --concern storage --concern storage-pressure
```

The battle-tested selector routed BT-001 through BT-004. It also reported
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
