# Edge Authentication and Calibration Concurrency Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove login-ID timing enumeration and enforce calibration `If-Match` atomically in both storage backends.

**Architecture:** The production web adapter keeps one dummy Argon2id hash so every credential attempt performs one bounded verification. Calibration expected revisions flow from the route-specific HTTP precondition through the application service into a guarded storage update whose history, boundary, and audit writes share the same transaction.

**Tech Stack:** Rust, Axum, Argon2id, SQLx, SQLite, PostgreSQL, Tokio tests.

## Global Constraints

- Do not change authorization roles, session policy, or login response bodies.
- Keep SQLite and PostgreSQL behavior equivalent.
- Keep calibration mutation and audit writes atomic.
- Add tests before production changes and observe each regression fail for the intended reason.

---

### Task 1: Equalize login verification work

**Files:**
- Modify: `edge/src/composition/web.rs`
- Test: `edge/tests/web_application_contract.rs`

**Interfaces:**
- Consumes: `hash_password`, `verify_password`, `Password`, and `PasswordHash`.
- Produces: `StorageWebApplication::login` behavior that invokes one Argon2 verification for both existing and missing accounts.

- [ ] Add a regression test that observes password-verification execution for a known and an unknown login ID.
- [ ] Run `cargo test -p iotkit-edge --test web_application_contract unknown_login` and confirm the unknown-login assertion fails.
- [ ] Add one dummy PHC to `StorageWebApplication` construction and select it on account lookup failure.
- [ ] Run the focused web application contract and confirm it passes.

### Task 2: Add atomic calibration revision checks

**Files:**
- Modify: `edge/src/composition/web.rs`
- Modify: `edge/src/application/semantics.rs`
- Modify: `edge/src/storage/semantic_output/operations.rs`
- Test: `edge/tests/web_application_contract.rs`
- Test: `edge/tests/output_contract.rs`

**Interfaces:**
- Consumes: parsed HTTP `If-Match` revisions and `StorageError::RevisionMismatch`.
- Produces: `update_calibration_as(..., expected_revision: Option<i64>, now)` and backend updates guarded by `calibration_revision`.

- [ ] Add SQLite and conditional PostgreSQL tests that submit two calibration updates with the same expected revision and assert the second fails without mutation.
- [ ] Run focused tests and confirm both fail because calibration currently has no revision precondition.
- [ ] Resolve calibration route preconditions from `semantic_signals.calibration_revision`.
- [ ] Thread the expected revision through `Semantics` and guard each backend update atomically.
- [ ] Run focused SQLite and configured PostgreSQL tests and confirm they pass.

### Task 3: Verify and commit

**Files:**
- Modify: only files listed above.

**Interfaces:**
- Consumes: completed regression fixes.
- Produces: a clean, committed security hardening change.

- [ ] Run `cargo fmt --check`.
- [ ] Run the focused authentication, HTTP, semantic/output, and storage test targets.
- [ ] Run `cargo clippy -p iotkit-edge --all-targets -- -D warnings`.
- [ ] Review `git diff --check` and `git status --short`.
- [ ] Commit with a security-fix message and report the SHA and exact verification commands.
