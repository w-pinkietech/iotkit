# Battle-tested review perspective

[日本語](README.ja.md) | English

This directory is the **first perspective** of the
[IoTKit review suite](../README.md): a small, repository-specific index of
operational failures that IoTKit changes must not accidentally reintroduce. It is
not the whole review process, a product contract, an incident database, a feature
backlog, or a claim that IoTKit has survived every listed condition. The skill
under `.agents/skills/iotkit-battle-tested-review/` is an execution aid for this
perspective only.

`catalog.json` is the only source of review entries. Codex and human reviewers use
the selector instead of loading the entire catalog:

```bash
node scripts/battle-tested-review.mjs select --base origin/master
node scripts/battle-tested-review.mjs select --base origin/master \
  --concern custody
node scripts/battle-tested-review.mjs concerns
```

Path routing is a lower bound. Add `--concern` when a semantic change affects a
public contract, authentication, custody, data loss, migration, restore, or an
external action even if its path does not select an entry. Unmatched paths and no
selected entries never mean that a change is safe.

## From field report to durable evidence

1. **Redact first.** Remove credentials, keys, tokens, customer and factory names,
   network identifiers, serial numbers, MQTT topics and payloads, database files,
   raw configuration, and screenshots containing those values.
2. **Triage the report.** Check for duplicates, establish impact and reproducibility,
   and decide whether it belongs to IoTKit. A report is not confirmed evidence.
   Close it with one recorded disposition: duplicate, needs information,
   accepted into the catalog, fixed or guarded, accepted risk with no change,
   out of scope, or routed to the security process. Do not open a catalog change
   for every report.
3. **Choose the smallest durable outcome.**
   - Keep a plausible but unconfirmed failure as `hypothesis`.
   - Mark it `field-reported` only while the report is still unconfirmed.
   - Use `field-observed` for a maintainer-confirmed occurrence.
   - Use `reproduced` when a controlled reproduction exists.
   - Add a focused regression test for repeatable product behavior.
   - Link a runbook for a failure that must be handled operationally.
4. **Update the index only when it improves future review.** Link the originating
   issue or existing repository evidence. Do not copy test procedures or runbooks
   into the entry.
5. **Delete or merge entries that no longer help reviewers.** Stable IDs exist for
   traceability, not permanent accumulation.

A catalog entry is a review question, not authorization to add a product feature.
Hypotheses require reproduction or a credible high-impact loss path before they
become implementation work.

## Catalog rules

- Keep one failure mode per entry and keep its question short.
- Do not use a catch-all path prefix.
- `provenance` and `guards` link to existing files or GitHub issues.
- A capacity test is not automatically a disk-full test.
- Sensor-device replacement is not Edge Node computer replacement.
- Normal pull-request CI checks catalog structure and routing logic only. Heavy
  outage and release gates remain focused or release-time tests.

Validate this perspective with:

```bash
node scripts/battle-tested-review.mjs check
node --test scripts/tests/battle-tested-review.test.mjs
```
