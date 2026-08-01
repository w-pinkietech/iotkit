# superpowers/specs → OKF gap inventory

**Status:** working survey for [#143](https://github.com/w-pinkietech/iotkit/issues/143)  
**As-of:** 2026-08-01  
**Authority:** none. Not product documentation. Current authority remains
`docs/okf/` plus paired contracts, fixtures, and tests.

## Policy context ([#145](https://github.com/w-pinkietech/iotkit/issues/145))

- **`docs/superpowers/` is kept** for clear design/plan lineage. Do not bulk-delete
  or rename as a priority. It is **not** product authority.
- **Do not add new specs/plans** here by default. Reuse the *writing style*
  (goal, non-goals, decision, verification; plan only when needed) on AGENTS
  change lanes instead.
- **Primary absorb path is redesign → OKF** ([#141](https://github.com/w-pinkietech/iotkit/issues/141)),
  not superpowers. This inventory is secondary: Console/operator model gaps if
  OKF stays too thin.
- Plans remain execution logs only—never normative.

## Purpose

Classify each file under `docs/superpowers/specs/` against current OKF and
shipped product behavior so later work can **rewrite verified gaps into OKF**—
not paste specs, and not treat plans as design.

**Out of scope:** `docs/superpowers/plans/` (execution checklists), redesign
(see #141 / `docs/redesign/okf-gap-inventory.md` if present).

## Method

| Field | Meaning |
|---|---|
| Still true? | Decision core still matches product / code / OKF? |
| In OKF? | Covered by `docs/okf/{ja,en}` (or top-level SECURITY/RELEASING where noted)? |
| Normative now? | Operators/implementers must follow it *today*? |
| Action | `skip` · `leave-evidence` · `absorb-okf` · `needs-verify` · `not-okf` |

- **absorb-okf** — rewrite into OKF (ja+en, revision++); verify against code.
- **leave-evidence** — keep as dated design log; do not maintain as current law.
- **needs-verify** — likely partially true; code/OpenAPI/e2e check before absorb.
- **not-okf** — process/tooling/repo ops; belongs in AGENTS, CI, SECURITY, RELEASING—not product OKF.
- **skip** — superseded or fully represented elsewhere.

Stale surface is expected: `IoTKit Site`, Go Site, old paths. Do not promote
unchanged paragraphs.

---

## Executive summary

| Bucket | Specs (approx.) | Implication |
|---|---|---|
| **Product UX / Console journey** | ~10 | Thickest gap vs OKF. Best absorb candidates. |
| **Naming / topology vocabulary** | 2 | Mostly in OKF; Site→Edge rename superseded earlier Site naming. |
| **Output / delivery product model** | 2 | Partial OKF (output-adapter, product-model); Console-facing model thin. |
| **Custody / recovery** | 2 | Largely in OKF recovery + install; design is longer narrative. |
| **Rust Edge rewrite / composition / CLI** | 3 | Implemented; architecture covers map; leave as evidence. |
| **Repo process** | ~5 | CI, release, security policy, resilience *test* matrix—not OKF product. |

**Volume insight:** Specs are often more detailed than OKF on **Console operator
capabilities**. OKF is stronger on **contracts, custody, install/recovery**.
Neither tree alone is complete.

**Do not absorb:** plans, Go-era resilience test designs as product law, full
pixel-level UI specs unless elevated to durable operator Guide.

---

## Priority absorb candidates (from specs)

Recommend separate child issues later. Prefer **durable operator/product
concepts**, not one-off layout bugs.

| ID | Topic | Source specs (primary) | Suggested OKF home | Confidence |
|---|---|---|---|---|
| S1 | Console purpose & operator journey (discover → configure → verify → export; where stuck) | `site-console-operator-journey`, `site-console-api` | Concept or new Guide-like Concept “Console operator model” | High value; **needs-verify** vs current routes/roles |
| S2 | Equipment hierarchy mental model (Edge Node ≠ device ≠ sensor; commissioning path) | `equipment-hierarchy`, `equipment-master-detail`, naming specs | Concept | High; aligns product-model + install |
| S3 | Sensor work surface (list/detail, live preview, rule categories) | `sensor-master-detail`, `sensor-editor-simplification`, `sensor-rule-preview-selection` | Concept or operations “Console” section—keep short | Medium; UI churn risk |
| S4 | Site-wide output destinations (not per-rule routes; no broker secrets in Console) | `site-wide-output-profiles`, `console-output-delivery-status` | Concept + cross-link output-adapter-v1 | High product value |
| S5 | Console non-goals (no credential surfaces, no broker provision, thin client) | journey, hardening, D13-era api design | Fold into S1 or product-model | High; overlaps redesign G5 |
| S6 | History / CSV as generic Observation export (not business reports) | `site-processed-history-csv` | operations or Concept one section | Medium—install already mentions CSV |

**Defer / verify hard:** responsive layout (S may be test evidence only), auth
concurrency hardening (security property—maybe architecture note if still true).

**Not OKF:** selective-ci, source-release-versioning, public-security-reporting
(SECURITY.md), rust-edge-* rewrite epics, resilience-matrix as Go test plan.

---

## Per-spec table

Paths are under `docs/superpowers/specs/`.

### Naming & product vocabulary

| Spec | Still true? | In OKF? | Normative? | Action | Notes |
|---|---|---|---|---|---|
| `2026-07-13-iotkit-edge-site-naming-design` | **Superseded** (Edge/Site pair) | Partial old | No | **leave-evidence** | Replaced by Edge Node / IoTKit Edge naming |
| `2026-07-21-iotkit-edge-node-naming-design` | **Yes** (current hierarchy) | **Yes** terminology + product-model | Yes via OKF | **skip** | Keep as rename rationale evidence |

### Console & operator experience

| Spec | Still true? | In OKF? | Normative? | Action | Notes |
|---|---|---|---|---|---|
| `2026-07-15-site-console-api-design` | **Partial** (baseline + later revisions) | **Thin** | Partial | **absorb-okf** via **S1/S5** after verify | Large; Site-era names; OpenAPI is machine peer |
| `2026-07-18-site-console-operator-journey-design` | **Partial–Yes** (intent) | **Thin** | Intent yes | **absorb-okf** **S1** | Best single “why Console exists” source |
| `2026-07-18-site-console-equipment-hierarchy-design` | **Partial–Yes** | Thin (install commissioning bullets) | Yes if shipped | **absorb-okf** **S2** | |
| `2026-07-18-site-console-equipment-master-detail-design` | **Partial** | No detailed | UX | **needs-verify** → maybe fold into S2 | Layout-level |
| `2026-07-18-site-console-sensor-master-detail-design` | **Partial–Yes** | Thin | UX | **absorb-okf** **S3** (short) | |
| `2026-07-20-site-console-sensor-editor-simplification-design` | **Partial** | No | UX polish | **leave-evidence** or tiny S3 note | Easy to rot |
| `2026-07-28-console-sensor-rule-preview-selection-design` | **Partial–Yes** if #89 shipped | No | Behavior | **needs-verify** → S3 | Bugfix design |
| `2026-07-28-console-responsive-layout-design` | **Partial** | No | UX | **leave-evidence** | Device breakpoints; not product law |
| `2026-07-24-console-hardening-design` | **Partial** | Partial (auth ops) | Security UX | **needs-verify** → S5 | CSRF/session etc. may be code-only |
| `2026-07-27-console-output-delivery-status-design` | **Partial–Yes** (#101) | Thin | Operator-facing | **absorb-okf** **S4** | |

### Output & data products

| Spec | Still true? | In OKF? | Normative? | Action | Notes |
|---|---|---|---|---|---|
| `2026-07-20-site-wide-output-profiles-design` | **Yes** (model) | **Partial** output-adapter-v1 | Yes | **absorb-okf** **S4** | Console terms vs internal model |
| `2026-07-21-site-processed-history-csv-design` | **Partial–Yes** | Thin (install mentions CSV) | Yes if shipped | **absorb-okf** **S6** | Generic Observation export |

### Custody / recovery / resilience

| Spec | Still true? | In OKF? | Normative? | Action | Notes |
|---|---|---|---|---|---|
| `2026-07-29-edge-node-computer-replacement-design` | **Partial–Yes** (slice shipped) | **Yes** recovery-v1 + install/hardware recovery | Yes via contracts | **skip** / optional narrative cross-link | Full design > slice-1; don’t re-import unshipped epochs |
| `2026-07-14-site-resilience-matrix-design` | **Stale process** (Go Site tests) | No as product | No | **leave-evidence** | Host resilience *scripts* may still exist; not OKF Concept |

### Edge service rewrite & engineering

| Spec | Still true? | In OKF? | Normative? | Action | Notes |
|---|---|---|---|---|---|
| `2026-07-24-rust-edge-replacement-design` | Done direction | Architecture crate map | Historical | **leave-evidence** | Rewrite epic |
| `2026-07-24-rust-edge-runtime-composition-design` | **Yes** if still matches serve | Partial architecture | Impl detail | **leave-evidence** | Composition root; rare OKF need |
| `2026-07-24-rust-edge-cli-parity-design` | **Partial** | install CLI mentions | Ops | **needs-verify** | Prefer runbook accuracy over absorb |
| `2026-07-24-edge-auth-concurrency-hardening-design` | **Partial** | Thin | Security | **needs-verify** | Only absorb proven invariants |
| `2026-07-24-adapter-author-onboarding-design` | **Partial** | input-adapter-v1 + adapter READMEs | Author DX | **needs-verify** | May stay outside OKF (developer guide in tree) |

### Repository / process (not product OKF)

| Spec | Still true? | In OKF? | Normative? | Action | Notes |
|---|---|---|---|---|---|
| `2026-07-23-selective-ci-design` | Evolved (later PRs) | No | CI | **not-okf** | Workflow + AGENTS/verify |
| `2026-07-27-source-release-versioning-design` | **Partial** | No (RELEASING/README) | Release process | **not-okf** | Keep RELEASING.md authority |
| `2026-07-29-public-security-reporting-design` | **Yes** if SECURITY.md current | SECURITY.md | Reporting policy | **not-okf** | Already top-level |

---

## Relationship to redesign inventory (#141)

| redesign gaps | specs overlap |
|---|---|
| G5 Console thin client / secrets | **S5**, journey, hardening |
| G1 series identity | Rarely in specs; still redesign/architecture |
| G4 R-map | Not in specs |
| D12 southbound | Barely; adapter onboarding mentions care incomplete |

**Conclusion:** Specs dominate **Console / output operator model** gaps. Redesign
dominates **platform identity / custody skeleton** gaps. Both feed OKF; different
chapters.

---

## What not to do

1. Do not copy Site-era names into OKF without Edge Node / IoTKit Edge mapping.
2. Do not turn layout pixel specs into Contracts.
3. Do not absorb plans’ checkbox steps.
4. Do not treat “Approved for implementation” as “still shipped exactly.”
5. Prefer OpenAPI + e2e + current templates as verification peers for Console absorbs.

---

## Suggested next steps

1. Merge this inventory (and #141 redesign inventory if open) as survey only.
2. First absorb PR from specs: **S1** (operator journey, 1–2 pages Concept) or **S4** (output destinations + delivery status model).
3. Parallel redesign absorb: **G1** series identity Why if still desired.
4. Optional: point `docs/superpowers/README.md` at this file (already historical).

## Read order until absorbs land

```text
1. docs/okf/                          current product law
2. edge/openapi + console code/e2e   Console “what is”
3. superpowers/specs (this inventory)  why / intended journey (distrust Site/Go)
4. superpowers/plans                 never normative
```
