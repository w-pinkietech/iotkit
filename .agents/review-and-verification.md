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

## Codex subagent split (optional Superpowers / multi-task plans)

When using project Codex agents, keep verification and review off the
implementer’s critical path:

| Concern | Owner |
|---|---|
| Implement settled task + focused tests | `implementer` |
| Fresh command evidence | `executor` |
| Independent findings (spec and/or quality) | `reviewer` (read-only) |

Orchestration, handoff checklist, and Superpowers skill mapping:
[`.codex/README.md`](../.codex/README.md).

Return to [`AGENTS.md`](../AGENTS.md).
