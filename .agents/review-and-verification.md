---
type: Guide
title: "Review and verification"
description: "Battle-tested review selection and risk-matched verification expectations."
language: en
translation_key: agents.review-and-verification
status: stable
revision: 1
---

# Review and verification

Before final review, use `$iotkit-battle-tested-review` or run the selector
directly. Review only selected `BT-NNN` entries plus semantic concerns that path
routing cannot infer. Zero selections and unmatched paths are not proof of safety.

Verification must match the changed failure paths. Run `scripts/verify.sh` when
Rust product behavior changes or cannot be excluded. Documentation-only changes
may use documentation, link, structure, and diff checks. When skipping a check
normally expected for the change, state the check and the concrete reason.

Tests passing are necessary, not sufficient: also compare the result with current
contracts and the [product invariants](product-invariants.md).

Return to [`AGENTS.md`](../AGENTS.md).
