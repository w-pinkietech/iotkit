# Common commands

Run the smallest command that can disprove the change, then widen for risk.

```bash
# Documentation, dependency, and source/test structure
npm ci --prefix scripts/docs  # after a fresh checkout or package-lock change
node scripts/check-product-docs.mjs
scripts/check-layers
scripts/check-source-layout

# Product-docs impact (which docs/product files might need a freshness update)
node scripts/product-docs-impact.mjs select --base origin/master
# Empty selection ≠ “no product-doc update needed”
# Soft freshness check (never fails; PR CI emits a warning when applicable)
node scripts/product-docs-impact.mjs soft-check --base origin/master --pr-body-file /tmp/pr-body.md

# Battle-tested review routing
node scripts/battle-tested-review.mjs select --base origin/master

# Rust focused / explicit workspace diagnosis
cargo test -p <crate-name>
cargo clippy -p <crate-name> --all-targets -- -D warnings
# Opt-in diagnosis only; not a routine PR command
scripts/verify.sh --workspace

# IoTKit Edge focused / full
cargo test -p iotkit-edge --test <contract-test>
cargo test -p iotkit-edge

# Console schema, generated assets, and browser journey
scripts/test-edge-console-frontend.sh
scripts/test-edge-console-e2e.sh

# Release-candidate host integration only
scripts/test-edge-host-release-gate.sh NEW_REPORT_DIRECTORY
```

For routine Rust work, use the affected package or contract check first. CI
selects the authoritative changed-scope Rust, Console, Edge, and trial lanes;
a local pass does not replace it. `scripts/verify.sh --workspace` runs Rust
formatting, layer rules, workspace tests, and Clippy with `-D warnings` only
when an explicit full-workspace diagnosis is useful. The host release gate and
field evidence are not per-PR defaults; see the
[verification ownership matrix](../.github/verification-ownership.md).
Raspberry Pi and physical sensors are required only when the task explicitly
requires hardware evidence.

Return to [`AGENTS.md`](../AGENTS.md).
