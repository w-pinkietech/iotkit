# Review and verification

Review is a **suite of perspectives**, not a single skill or catalog. Start at
[`review/README.md`](../review/README.md).

## Perspectives

1. Open the suite entry and pick the perspectives that match the change.
2. For product or operations-touching diffs, always consider the
   **battle-tested** perspective
   ([`review/battle-tested/README.md`](../review/battle-tested/README.md)).
   Use `$iotkit-battle-tested-review` or run the selector:

   ```bash
   node scripts/battle-tested-review.mjs select --base origin/master
   ```

3. Review only selected `BT-NNN` entries plus semantic concerns that path
   routing cannot infer. Zero selections and unmatched paths are not proof of
   safety.
4. When more perspectives exist under `review/`, apply those that fit. Do not
   treat the battle-tested catalog as the definition of all review.

The battle-tested skill
([`skills/iotkit-battle-tested-review/SKILL.md`](skills/iotkit-battle-tested-review/SKILL.md))
executes one perspective; it does not replace the suite entry.

## Verification

Verification must match the changed failure paths. Run `scripts/verify.sh` when
Rust product behavior changes or cannot be excluded. Documentation-only changes
may use documentation, link, structure, and diff checks. When skipping a check
normally expected for the change, state the check and the concrete reason.

Tests passing are necessary, not sufficient: also compare the result with current
contracts and the [product invariants](product-invariants.md).

## Codex subagent split (optional Superpowers / multi-task plans)

When using project Codex agents, keep acceptance verification and review outside
the implementation agent’s ownership:

| Concern | Owner |
|---|---|
| Implement routine settled task + focused tests | `implementer` |
| Implement context-heavy or higher-risk settled task + focused tests | `complex_implementer` |
| Fresh command evidence and acceptance | Main |
| Independent findings (spec and/or quality) | `reviewer` (read-only) |

Orchestration, handoff checklist, and Superpowers skill mapping:
[`.codex/README.md`](../.codex/README.md).

Return to [`AGENTS.md`](../AGENTS.md).
