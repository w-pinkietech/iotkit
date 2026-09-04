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

Verification is the acceptance evidence the issue names: the journey stage that
must pass and the unit tests that stay ([`testing.md`](testing.md)). For
immediate feedback, run the owning package's tests and lint; CI (lightweight
checks plus the full Rust workspace, and the journey once it exists) is the
authoritative merge evidence. `scripts/verify.sh --workspace` is an opt-in
cross-workspace diagnosis. Documentation-only changes may use documentation,
link, structure, and diff checks. When skipping a check normally expected for
the change, state the check and the concrete reason.

Tests passing are necessary, not sufficient: also compare the result with current
contracts and the [product invariants](product-invariants.md).

## Codex implementation roles

For every routine Codex task, keep acceptance verification outside the
implementation agent’s ownership. Select the implementation role by task shape:

| Concern | Owner |
|---|---|
| Implement routine settled task + the tests the issue names | `implementer` |
| Implement context-heavy or higher-risk settled task + the tests the issue names | `complex_implementer` |
| Fresh command evidence and acceptance | Main |
| Independent findings (spec and/or quality) | `reviewer` (read-only) |

Independent review is for work that touches a public contract or can lose data.
For #232 that is child issues 1 (contract), 3 (core), and 4 (MQTT Output
Adapter); TOML, deletion, and Console (2, 5, 6) are accepted on Main's diff
inspection and CI. Main retains orchestration, architecture, policy, and final
acceptance. If review requests changes, return to the selected implementation
role, rerun Main's fresh verification, and obtain a new independent review.

Orchestration, handoff checklist, and Superpowers skill mapping:
[`.codex/README.md`](../.codex/README.md).

Return to [`AGENTS.md`](../AGENTS.md).
