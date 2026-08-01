---
type: Runbook
title: "Optional OKF v0.2 provenance and trust metadata"
description: "When and how to add sources, generated, and verified on product docs without making them mandatory."
language: en
translation_key: operations.okf-optional-meta
status: stable
revision: 1
---

# Optional OKF v0.2 provenance and trust metadata

Status: Guidance for authors. These fields are **optional**. Existing concepts
without them remain valid under the IoTKit product gate.

Process-level freshness (same-PR product-doc updates, bilingual `revision`) stays
mandatory. Frontmatter metadata here is an extra, machine-readable trust signal
for agents and reviewers.

## When to add metadata

When **lasting product facts** change, consider adding these optional OKF
families; if they are already present, refresh them:

- Public wire or custody **contract** text
- Operator **runbook** steps that change a field procedure
- Architecture claims that redefine component ownership

Skip metadata when the change is:

- Typo, link fix, or formatting only
- Internal refactor with no product-visible fact change
- Pure translation alignment that does not alter meaning (still bump `revision`)

## Minimal examples

Keep IoTKit required scalars. Nest OKF families as full YAML (the checker accepts them).

```yaml
---
type: Contract
title: "…"
description: "…"
language: en
translation_key: contracts.example
status: stable
revision: 4
generated: { by: human:your-handle, at: 2026-08-01T12:00:00Z }
verified: { by: human:your-handle, at: 2026-08-01T12:30:00Z }
sources:
  - id: schema
    resource: https://example.invalid/path-to-schema-or-fixture
    title: Contract schema, fixture, or conformance test; or dated design evidence
---
```

Actor convention (OKF):

- `human:<id>` for people
- `process:<id>` for automation
- `<producer>/<version>` for agents and tools

`verified` may be a single mapping or a list. Prefer `human:` when a person
confirmed the definition against code or co-authority artifacts.
Within a family that is present, every source has `resource`, `generated` has
`by`, and each verification event has `by` and `at`; `at` uses an ISO 8601 datetime.

## Coexistence with IoTKit required keys

| Always required (IoTKit gate) | Optional (OKF v0.2) |
|---|---|
| `type`, `title`, `description`, `language`, `translation_key`, `status`, `revision` | `sources`, `generated`, `verified`, `stale_after`, … |

Do not remove required keys to “simplify” to plain OKF. The producer profile is
intentional (see [bundle root](../../index.md)).

## Pilot policy

Bulk backfill of all concepts is **out of scope**. Prefer adding metadata on the
next substantive edit of high-value contracts and runbooks. A broader pilot can
be a separate issue.

## Related

- Bundle layers and intentional OKF differences: [product index](../../index.md)
- Official OKF v0.2: https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md
- Process freshness: repository `AGENTS.md` (**Keep product docs current**)
