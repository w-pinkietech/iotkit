---
name: iotkit-battle-tested-review
description: Use when reviewing an IoTKit pull request for operational failure risks, triaging an IoTKit field report, or deciding whether field evidence should update a review entry, regression test, or runbook.
---

# IoTKit Battle-tested Review

Use the catalog as an index, not a safety verdict or backlog. Tests and runbooks
remain authoritative.

## Pull request review

1. From the repo root, run:

   ```bash
   node scripts/battle-tested-review.mjs select --base <base-ref>
   ```

2. For cross-cutting risk, list concerns and rerun with relevant `--concern`:

   ```bash
   node scripts/battle-tested-review.mjs concerns
   ```

3. Review the diff against selected questions, guards, and coverage gaps.
   Unmatched paths, concerns without entries, and zero selections are not proof
   of safety.
4. Record selected `BT-NNN` IDs or a reason for none. Run focused tests for the
   changed behavior, not every catalog scenario.

## Field report triage

Follow `SECURITY.md`, the field-report Issue form, and
`review/battle-tested/README.md`. Redact before copying. Separate reported facts
from confirmed evidence, and reuse an entry for the same failure.

Promote the smallest durable outcome: a review question for credible unconfirmed
risk, a focused regression test for reproducible behavior, or a runbook link for
verified operations. Never implement a feature solely from a hypothesis or one
report.
