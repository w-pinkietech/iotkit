# IoTKit review suite

[日本語](README.ja.md) | English

`review/` is the source of truth for **how this repository reviews changes**.
It is a suite of **perspectives**, not a single skill, a single catalog, or a
product contract.

A perspective answers four questions:

| Element | Meaning |
|---|---|
| **Intent** | What risk or quality concern it focuses on |
| **When** | When to apply it (every PR, path/concern match, Full-lane only, …) |
| **How** | Checklist, catalog, selector, or short procedure |
| **Not** | What it does **not** prove (especially: not a safety certificate) |

Machine path selectors (like battle-tested routing) are optional. They give a
**lower bound**. Empty selection, unmatched paths, and green CI never mean a
change is safe.

Product authority remains `docs/product/` (OKF packaging). This suite is process
and quality review, not a second product corpus.

## Perspectives

| Perspective | Status | Intent (short) | Entry |
|---|---|---|---|
| **battle-tested** | Active — first perspective | Operational failures IoTKit must not reintroduce; field evidence triage | [battle-tested/README.md](battle-tested/README.md) |
| **pure-refactoring** | Experimental — report-only | Measure a blinded synthetic evaluator; never evidence or merge authority | [pure-refactoring/README.md](pure-refactoring/README.md) |

Future perspectives (not defined here yet) may cover secrets handling,
issue-scope drift, public contracts, Console operator journeys, or layer rules.
Add them as sibling directories under `review/` with the same Intent / When /
How / Not shape. Do not fold every concern into the battle-tested catalog.

## How to use the suite

1. Open this file and pick the perspectives that match the change.
2. Always consider **battle-tested** for product or ops-touching diffs; run its
   selector when paths or concerns may match:

   ```bash
   node scripts/battle-tested-review.mjs select --base origin/master
   ```

3. Apply any other active perspectives listed above. The experimental
   **pure-refactoring** perspective is only for evaluating its own rubric and
   captured runs; it is never a normal PR gate or proof of equivalence.
4. Record selected IDs (for example `BT-NNN`) or a concrete reason that none
   apply. Record semantic concerns path routing cannot see.
5. Match verification to the failure paths you reviewed
   ([`.agents/review-and-verification.md`](../.agents/review-and-verification.md)).

Agent skill for the battle-tested perspective:
[`.agents/skills/iotkit-battle-tested-review/SKILL.md`](../.agents/skills/iotkit-battle-tested-review/SKILL.md).
The skill is an **execution aid** for one perspective; it does not define the
whole review suite.

## Common rules

- **Zero selection ≠ safe.** Reviewers still own semantic and contract risk.
- A catalog entry or checklist item is a **review question**, not permission to
  ship a feature or claim production readiness.
- Do not grow a catch-all catalog. Prefer a new perspective when the intent
  differs (for example secrets vs operational failure modes).
- Redact credentials, customer identity, and raw field artifacts before they
  enter issues, PRs, or catalog links.
