# redesign → OKF gap inventory

**Status:** working survey for [#141](https://github.com/w-pinkietech/iotkit/issues/141)  
**As-of:** 2026-08-01  
**Authority:** none. This file is not product documentation. Current product
authority remains `docs/okf/` plus paired contracts, fixtures, and tests.

## Policy context ([#145](https://github.com/w-pinkietech/iotkit/issues/145))

- **OKF** is the only human-readable product authority.
- **`docs/superpowers/` stays.** Clear lineage as sprint design/plans; do not
  bulk-delete or rename. Writing *style* may be reused on light lanes; trees
  are not current law. Specs→OKF absorbs are secondary ([#143](https://github.com/w-pinkietech/iotkit/issues/143)).
- **`docs/redesign/` is the main risk and the main absorb path.** It is easy to
  misread as a second current corpus. Fix by rewriting still-true gaps into OKF
  (this inventory’s G1–G7), not by “updating” dated evidence to match today.
- Do not paste redesign wholesale. Verify against code/contracts first.

## Purpose

Classify `docs/redesign/` material against current OKF so later work can
**rewrite only still-true, still-normative gaps into OKF**—not copy redesign
text wholesale, and not “fix” historical evidence to match today.

## Method

For each item:

| Field | Meaning |
|---|---|
| Still true? | Does the *decision core* still match code / OKF / product intent? |
| In OKF? | Covered by current `docs/okf/{ja,en}` (and paired contracts)? |
| Normative now? | Should operators/implementers treat it as a must-follow rule *today*? |
| Action | `skip` · `leave-evidence` · `absorb-okf` · `needs-code-first` · `defer` |

Actions for a later phase (not this survey):

- **absorb-okf** — write fresh OKF prose (both languages, revision++), verify
  against code; do not paste redesign as-is.
- **leave-evidence** — keep under redesign as dated material; do not maintain
  as current.
- **needs-code-first** — design exists but product is not ready for a contract.
- **defer** — true but lower priority or future scope.
- **skip** — obsolete or already fully represented; do not absorb.

`docs/superpowers/` is out of scope for this inventory.

---

## Executive summary

| Bucket | Count (approx.) | Meaning |
|---|---|---|
| Already in OKF (core) | Many of D1, D7–D11, D8–D9, parts of D2/D4/D5 | Custody, ingest, adapters, product model, architecture map |
| Gap candidates for OKF rewrite | Few focused topics | See “Priority absorb list” |
| Deferred / not normative yet | D12 bulk, full R1–R23 map, Wave process | Do not contract-ize unfinished design |
| Evidence only (will not match current product) | inputs, reviews, rewrite-prep, adr-inventory | Correct *as snapshots*; wrong *as current corpus* |

**Main finding:** Large parts of redesign *decisions* were already distilled into
OKF contracts and architecture. What remains is not “move the tree,” but a
**small set of concepts** still thin in OKF, plus **lots of dated evidence** that
must stay non-authoritative.

**Secondary finding:** Even “still true” decisions contain **stale surface**
(Wave labels, Go Edge, host-agent, gateway wording). Absorption must re-verify
and rewrite; never promote redesign paragraphs unchanged.

---

## Priority absorb list (candidates only)

Ordered by (value × confidence that content is still true × OKF thinness).
Each becomes its **own later issue/PR** after human pick.

| ID | Topic | Suggested OKF type | Why gap | Still true? (provisional) |
|---|---|---|---|---|
| G1 | Device / series identity narrative (rename-safe, replace-hardware continues history) | Concept (or Architecture subsection expansion) | Architecture has a bullet; terminology has only a one-line `series`; D5 “why” (legacy label-as-key failure) is thin | Yes (matches architecture + install replace-hardware wording) |
| G2 | Measurement registry two-layer (standard catalog vs field registry; accept only field) | Concept or Architecture | Architecture mentions registry crate; D6 policy/rationale not a first-class concept | Partial—needs code check of registry crate vs D6 |
| G3 | Adapter package anatomy (transport / driver / runtime / composition / ingest client) | Concept or Architecture | input-adapter-v1 is northbound host contract; D4 composition story is only partly mirrored | Partial—northbound yes; full D4 map uneven |
| G4 | Edge Node responsibility map **as implemented** (not full R1–R23 wishlist) | Architecture | Ledger R1–R23 is redesign-only; architecture has crate map + control plane but not R-index | Partial—implement slice only |
| G5 | Console / operator surface boundaries (thin client, no secrets, no broker provision) | Concept or operations note | Scattered in product-model + install; D13 as a single “UI scope” is clearer | Yes for shipped Console rules |
| G6 | Ingress threat model & traffic classes (accident-over-malice, capacity as operator duty) | Contract annex or operations—only if still enforced | ingest-v1 has tokens/curl; D11 threat stairs / flow classes may exceed shipped behavior | Partial—**verify code before absorb** |
| G7 | Southbound care scope **non-goals** (actuation out; AdapterCommand frozen legacy) | Concept one-liner or architecture | architecture already says frozen southbound vocab; D12 full care contract is not product | Non-goals yes; full D12 **needs-code-first** |

**Not on absorb list:** wholesale D3 Wave schedule, monojoh ADR inventory, YokaKit topic catalog, host-agent era diagrams.

---

## A. Decision corpus (`decisions/`, terminology, ledger)

| Artifact | Still true? | In OKF? | Normative now? | Action | Notes |
|---|---|---|---|---|---|
| **D1** ingest model | **Yes** (core: envelope/ack, dedup sender+id, no central transform) | **Yes** — ingest-v1, architecture collector path | Yes via contracts | **skip** (already absorbed) | Stale: Wave/deferred nuance in long body—do not re-import |
| **D2** authority, commissioning, recovery image | **Partial** | **Partial** — product-model, architecture, custody, recovery, install | Yes where OKF/ops say so | **absorb-okf** only if specific missing commissioning invariants found; else **skip** | Many ops details live in install/recovery runbooks |
| **D3** process & waves | **Partial** (product value / v1 goal still useful) | **No** as process | **No** (status/waves not authority) | **leave-evidence** or later optional Concept “product value” if not redundant with product-model | Do not absorb Wave tables |
| **D4** adapter anatomy | **Partial** | **Partial** — input-adapter-v1, crate map | Northbound yes | **absorb-okf** candidate **G3** | Care-servicer half points at D12 |
| **D5** series identity | **Yes** (system_id / hardware_id / user_label) | **Partial** — architecture bullet; not a Concept | Yes in implementation | **absorb-okf** candidate **G1** | Strong “why” still mostly in D5 |
| **D6** measurement registry | **Partial** | **Partial** — registry crate mentioned | If code matches two-layer | **absorb-okf** candidate **G2** after code check | |
| **D7** exit / upstream contract | **Yes** (raw stream, no business events in core) | **Yes** — custody-v1, output-adapter-v1, product-model | Yes | **skip** | D7 body still has Wave-0 schedule noise |
| **D8** multi Edge Node topology | **Yes** (each Pi full node; no central collector) | **Yes** — product-model, architecture, activation | Yes | **skip** | |
| **D9** MQTT binding | **Yes** | **Yes** — custody-v1 topics | Yes | **skip** | |
| **D10** exit auth / path | **Yes** (static creds, ACL, TLS, no secret leakage) | **Partial–Yes** — architecture bootstrap, install, custody ACL | Yes | **skip** or tiny ops cross-link if a rule is only in D10 | Prefer verify vs `bootstrap-edge` / install |
| **D11** ingress auth / admission | **Partial** | **Partial** — ingest-v1 | Only shipped HTTP path | **absorb-okf** only after code check **G6**; else **leave-evidence** | Pairing windows / flow classes may exceed v1 |
| **D12** southbound care | **Partial** (scope “care only, no actuation” intent) | **Minimal** — frozen AdapterCommand; no care contract | **No** full contract | **needs-code-first** for full care; **G7** for non-goals only | Largest “looks like design but not product law” risk |
| **D13** UI scope | **Yes** (thin UI, no secret surfaces, Console ≠ broker provision) | **Partial** — product-model, install | Yes for Console | **absorb-okf** candidate **G5** | |
| **terminology.md** (redesign) | **Partial** (4-tier + bans useful; “Edge is Go” false) | **Yes** thinner OKF terminology | OKF terms yes | **skip** OKF as current; redesign = evidence of older fuller glossary | Optional later: expand OKF terminology from *verified* rows only |
| **responsibility-ledger.md** R1–R23 | **Partial** (mix of done / future) | **No** as R-index | **No** as full list | **absorb-okf** candidate **G4** as *implemented* map only | Do not paste R1–R23 wholesale into OKF |

---

## B. Evidence / provenance (category C)

These **will not match** current IoTKit organization. That is expected.

| Artifact | Still true? | In OKF? | Normative now? | Action | Stale examples |
|---|---|---|---|---|---|
| **inputs/yokakit-consumer-catalog** | True *as 2026-07-03 YokaKit wire extract* | No (must not) | **No** | **leave-evidence** | Flat topics `production`/`alarm`; `ipAddress`+`pinNumber`; no device timestamps; anonymous MQTT |
| **reviews/** (exit, topology, codex) | True *as review minutes* | No | **No** | **leave-evidence** | Topology review Status “未決” while D8 later settled Model B-class; “gateway” wording |
| **rewrite-prep.md** | True *as 2026-07-01 workspace map* | No | **No** | **leave-evidence** | monojoh as binding start; iotkit-gateway; host-agent |
| **adr-inventory.md** | True *as monojoh ADR triage* | No | **No** | **leave-evidence** | States redesign is *current* design authority—**obsolete rule** |
| **diagrams/*.html** | Unverified visuals | No | **No** | **leave-evidence** | Not synced to OKF |

**Do not rewrite these to “match OKF.”** Do not absorb into OKF.

---

## C. Cross-cutting drift (all redesign)

| Drift type | Example | Implication for absorption |
|---|---|---|
| Stack | Edge “Go+SQLite” in redesign terminology | Rewrite from current Rust Edge |
| Process labels | Wave 0 / 計画4 / monojoh queues | Drop; use lanes / issues |
| Component names | gateway, host-agent, iotkit-site era | Use OKF terms only |
| Authority meta | “redesign is current authority” | False; OKF is current |
| Consumer wire | YokaKit flat MQTT in inputs | Application-specific; Output Adapter territory, not core exit |

---

## D. Suggested next steps (after this survey)

1. Human selects G1–G7 items to pursue (recommend start **G1** or **G5**—high confidence, small surface).
2. Per item: code skim → draft OKF ja+en → `node scripts/check-okf-docs.mjs` → PR closing a child issue.
3. Optionally add a one-line banner to redesign README pointing at this inventory and OKF (separate tiny PR).
4. Do **not** move/delete redesign or superpowers until several absorbs land and confusion drops.

---

## E. How to read redesign until then

```text
1. Current behavior / rules  → docs/okf/ (+ fixtures/tests)
2. Why (if needed)           → D* decision cores, distrust Wave/stack paragraphs
3. Dated evidence            → inputs/, reviews/, rewrite-prep, adr-inventory
```

If 2 conflicts with 1, **stop**; do not “fix” by reasserting redesign.
