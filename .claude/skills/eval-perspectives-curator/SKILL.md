---
name: eval-perspectives-curator
description: Use after a codex-eval-* / per-task cross-vendor review completes, when scripts/watchpoints.sh reports expired items, or when the user asks to audit review perspectives.
---

# Eval Perspectives Curator

Keeps the review guides' Active Watchpoints sharp. A watchpoint earns its place by
changing what the reviewer looks for; everything else is noise.

**Files:** `docs/eval/{spec,plan,impl-spec,impl-quality}-review.md` — each holds
Active Watchpoints (living) + Baseline Checklist (stable) + Maintenance rules.
This skill curates those four files only — never source code, specs, or plans.

## Triggers

1. **After each review cycle** — did the review expose a new blind spot? did an
   existing watchpoint prove itself (predicted a finding) or fail (missed one)?
2. **Expiry** — `scripts/watchpoints.sh` lists items past their Revalidate-by date.
   Adjudicate every listed item; never leave expired items sitting (this rotted
   silently for 11 days once — hence the script).
3. **User request** — full audit pass over all four files.

## Adjudicating a watchpoint

| Situation | Action |
|---|---|
| Validated by a recent finding | Renew (+3 months) |
| Relevant to upcoming work but never triggered | Renew ONCE with a dated note; delete at next expiry if still untriggered |
| Codebase moved on / condition gone | Delete |
| Covered by another watchpoint or the Baseline | Absorb into the stronger one, delete the weaker |
| Triggered 3+ times | Promote to Baseline Checklist, remove from Active |

## Adding a watchpoint

Register only what is project-specific, concrete, reproducible, and would change a
future review. Skip generic advice, one-off fix details, and anything a reviewer
without project context would already check.

Climb the generalization ladder to level 1–2 first:

- L0 (too narrow): "OPT3001 write needs LE bytes"
- L1: "I2C register writes must match the sensor datasheet byte order"
- L2: "every I/O boundary documents its byte-order contract"
- L3 (too broad): "write correct code"

**Test:** would this help review a DIFFERENT adapter/sensor/plan? No → too narrow.
Would a reviewer without project context already check it? Yes → too generic.

## Format (the one true shape — watchpoints.sh parses the date line)

```
- Added: YYYY-MM-DD
  Revalidate by: YYYY-MM-DD   (default +3 months)
  Watchpoint: <one testable claim — what to look for and why it bites>
  Observed in: <where it was seen>
```

- Max 10 per file; adding an 11th evicts the least valuable.
- Empty section reads `(none currently)`. No duplicates — check before adding.

## Balance check (end of any pass)

- 3+ watchpoints on one theme → merge the closest pair.
- A concern area (concurrency / I/O boundaries / config / testing / naming) with
  repeated findings but zero watchpoints → consider adding one.
