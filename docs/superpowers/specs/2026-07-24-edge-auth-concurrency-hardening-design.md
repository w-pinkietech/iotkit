# Edge Authentication and Calibration Concurrency Hardening

## Scope

Fix two security defects in the Rust Edge runtime:

1. Unknown login IDs must perform the same bounded Argon2id verification work as known login IDs before returning `invalid_credentials`.
2. Calibration mutations must compare `If-Match` with the calibration revision and enforce that comparison atomically in SQLite and PostgreSQL.

No other authorization, rate-limit, or revision behavior changes.

## Authentication design

`StorageWebApplication` owns one valid dummy password hash generated with the production Argon2id parameters. Login selects the stored credential when the normalized login ID exists and otherwise selects the dummy hash. Both cases execute `verify_password` exactly once. Unknown accounts still increment the same normalized failure bucket and return the same response as a wrong password.

The dummy hash is created when the application is composed, not per request. Its plaintext is a fixed internal value that is never an account credential, and the PHC remains redacted by existing secret wrappers.

## Calibration concurrency design

The web mutation adapter resolves `If-Match` according to the route. For `/api/v1/signals/{signal_ref}/calibration`, it reads `semantic_signals.calibration_revision`, not a semantic-rule revision.

The expected revision is passed through `Semantics` to storage. Each backend updates calibration with a predicate on the expected revision and returns `RevisionMismatch` when no row is updated. Revision increment, revision-history insertion, activation boundary capture, and audit insertion remain in one transaction.

Console mutations continue to use the current revision resolved inside the serialized application mutation boundary because console forms do not carry `If-Match`.

## Verification

- A login regression test uses an injected verification observer to prove both known and unknown login IDs execute one password verification without timing-based wall-clock assertions.
- SQLite tests issue two calibration writes with the same expected revision and prove the second is rejected without changing calibration or audit state.
- The PostgreSQL contract runs the same stale-writer assertion when the configured PostgreSQL test database is available.
- Existing authentication, HTTP, semantic, and PostgreSQL contract suites remain green.
