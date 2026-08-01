# Source and test placement

- Rust product `src/` contains product code, not test bodies or test helpers.
  Private-API tests live under `<crate>/tests/unit/**/*_tests.rs` and are included
  by a minimal `#[cfg(test)] #[path = "..."] mod tests;`. Public integration
  tests live under `<crate>/tests/*.rs`.
- Test constructors, clocks, fixtures, mocks, and observation helpers live in
  `tests/support/` or a dedicated testkit.
- Frontend unit tests live under `edge/frontend/tests/unit/`, not `src/`.
- `scripts/check-source-layout` enforces these boundaries.

Return to [`AGENTS.md`](../AGENTS.md).
