# Common commands

Run the smallest command that can disprove the change, then widen for risk.
What counts as acceptance evidence is in [`testing.md`](testing.md).

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

# Rust: owning package for immediate feedback; CI runs the full workspace
cargo test -p <crate-name>
cargo clippy -p <crate-name> --all-targets -- -D warnings
# Full-workspace diagnosis (fmt, layers, layout, tests, clippy)
scripts/verify.sh --workspace

# MQTT Output Adapter v1 contract: schema + canonical bytes, then the consumer
# side alone (fixtures -> Mosquitto -> subscriber). Needs mosquitto with
# mosquitto_pub/mosquitto_sub, or docker.
node scripts/check-observation-fixtures.mjs
scripts/test-observation-consumer.sh
# The journey (L1 minimal loop + L2 fault injection): builds iotkit-edge-node
# and nodectl, runs them against a throwaway Mosquitto, checks at an independent
# consumer. Needs mosquitto + mosquitto_pub/mosquitto_sub and python3. About
# 20 s after the build. IOTKIT_JOURNEY_BIN_DIR=<dir> skips the cargo build.
scripts/test-journey.sh

# Trial profile launcher (docker compose: Edge Node + Mosquitto)
python3 -m unittest scripts.tests.test_iotkit_trial
```

CI runs the lightweight, full Rust, and journey lanes on every PR; there is no
changed-path selection, and a local pass does not replace CI. Raspberry Pi and
physical sensors are required only when the task explicitly requires hardware
evidence (journey stage L4).

Return to [`AGENTS.md`](../AGENTS.md).
