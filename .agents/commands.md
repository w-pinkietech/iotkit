---
type: Guide
title: "Common verification commands"
description: "Smallest-first commands for docs checks, Rust tests, Console, and verify.sh."
language: en
translation_key: agents.commands
status: stable
revision: 1
---

# Common commands

Run the smallest command that can disprove the change, then widen for risk.

```bash
# Documentation, dependency, and source/test structure
node scripts/check-okf-docs.mjs
scripts/check-layers
scripts/check-source-layout

# Battle-tested review routing
node scripts/battle-tested-review.mjs select --base origin/master

# Rust focused / full
cargo test -p <crate-name>
scripts/verify.sh

# IoTKit Edge focused / full
cargo test -p iotkit-edge --test <contract-test>
cargo test -p iotkit-edge

# Console schema, generated assets, and browser journey
scripts/test-edge-console-frontend.sh
scripts/test-edge-console-e2e.sh

# Release-candidate host integration only
scripts/test-edge-host-release-gate.sh NEW_REPORT_DIRECTORY
```

`scripts/verify.sh` runs Rust formatting, layer rules, workspace tests, and
Clippy with `-D warnings`. The host release gate is not a per-PR default.
Raspberry Pi and physical sensors are required only when the task explicitly
requires hardware evidence.

Return to [`AGENTS.md`](../AGENTS.md).
