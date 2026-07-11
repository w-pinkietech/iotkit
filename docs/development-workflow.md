# Development Workflow

Status: **Authoritative** (2026-07-11)

This document is the single workflow authority for `iotkit-next`. Design authority remains
`../docs/redesign/`; structural authority remains `docs/architecture.md`. `CLAUDE.md`,
`AGENTS.md`, and `.claude/skills/` may add role-specific mechanics but must not contradict
this document.

The goal is not fewer quality gates. It is fewer avoidable waits, repeated investigations,
and human approvals while retaining independent review, TDD, verification, and design-corpus
integrity.

## 1. Main-agent responsibilities

The Main agent simultaneously acts as:

- **Flow Steward**: minimize critical-path wall time without weakening gates; dispatch
  independent work in parallel, freeze only artifacts under review, and measure rework.
- **Design Integrity Lead**: before recommending a design, verify canon alignment, state
  transitions, trust/secret provenance, recovery, and real user steps.
- **Autonomous Delivery Lead**: within an approved mission and the Green/Yellow envelope,
  continue through design, review reconciliation, implementation orchestration, verification,
  and commits without asking for piecemeal approval.

Main does not write Rust product code. Codex implementation workers do. Main may edit docs,
skills, scripts, CI, and other workflow plumbing.

## 2. Mission brief and risk classification

At the start of a plan or materially new workstream, record:

- objective and acceptance outcome;
- inviolable constraints;
- non-goals;
- affected design authorities and responsibilities;
- the Red decisions that still require human value judgment;
- direct consequences that Main may handle autonomously.

Classify the work by impact, not line count:

| Risk | Typical work | Required pipeline |
|---|---|---|
| Small | clear bug, wording, test, internal refactor inside settled design | reality check → failing test → fix → verify → targeted review → commit |
| Medium | config, non-destructive migration, internal API, local reversible feature | short design note → Design Ready → targeted three-vendor review → concise plan → implementation → integration review |
| Large / Red | auth, secrets, wire/API, custody, restore, threat model, major responsibility boundary | Mission Brief → Design Ready → spec → three-vendor review → contract-centered plan → plan review → task loops → final integration review |

Small requires at least one independent vendor; Main's own analysis never substitutes.
Medium and Large require all three vendors. A targeted review that discovers contract or
trust impact reclassifies the work before commit.

Any public-contract, auth/secret, data-loss/custody, restore, trust-boundary, irreversible
effect, review wrapper, settlement/verification logic, CI quality gate, or workflow authority
change mechanically promotes work to Large/Red. Plan 6 is Large/Red. Record the classification
and promotion check in the active ledger before dispatch.

## 3. Design Ready gate

Before presenting a preferred design, writing a spec, or requesting a Red decision, create a
compact evidence pack containing all of the following.

### 3.1 Constraint ledger

- relevant canon and responsibility entries;
- settled decisions and decisions being changed;
- non-goals;
- assumptions and unverified platform facts;
- explicit separation of facts, user-value judgments, and implementation choices.

### 3.2 State machine

Cover success plus concurrent requests, response loss, power loss around commit, restart,
unset/jumping/rollback clocks, expiry/lockout, local recovery, snapshot restore, factory reset,
and missing/corrupt DB where applicable.

### 3.3 Trust and secret provenance

For every credential, identity, and key: generator, first knower, transfer channel, trust
anchor, storage/log/audit representation, revocation, rotation, and restore behavior.

### 3.4 User journey

Count actual steps for every affected persona, including secrets entered, long strings,
certificate warnings, SSH/SD/reboots, recovery, one-box versus 20–100-box repetition, and the
old/low-end client target from `docs/architecture.md`.

### 3.5 Adversarial six

Answer at minimum:

1. Two identical requests arrive concurrently.
2. Power fails immediately before/after commit.
3. Commit succeeds but the success response is lost.
4. The clock jumps from an unset value to current time, or backward.
5. An old backup is restored to another box.
6. A same-LAN third party acts before the legitimate installer.

Add ENOSPC, network partition, sender rotation, or secret disclosure when relevant.

### 3.6 Traceability

Every important invariant maps to a mechanism, failure behavior, and verification method.
Any blank cell means the design is not ready.

## 4. Autonomous decision classes

Classification depends on impact, reversibility, and external effect—not model confidence.

### Green — autonomous

Settled-spec implementation details, internal APIs, module boundaries, tests, diagnostics,
lint/type safety, exact review fixes that preserve the approved contract, and documentation
alignment. Main proceeds through commit and reports at the milestone.

### Yellow — autonomous with record

Reversible dependency/config additions, non-destructive migrations, internal performance or
complexity tradeoffs, UX wording that does not change user steps or safety, and Minor triage.
Record rationale, rollback, affected scope, and reconsideration trigger in the active ledger.

Before every Yellow milestone commit, Main rolls up all mission Yellow decisions against every
Red axis and records whether the approved product envelope is unchanged. Required reviewers
independently confirm that classification. Individually reversible changes that cumulatively
alter product behavior are promoted to Red.

### Red — human decision

Threat/trust/auth strength changes; ack, custody, or data-loss contracts; breaking public
wire/API changes; irreversible migrations; restore/factory-reset semantics; material user or
product scope; canon conflicts not resolvable by authority order; and external publication,
push, PR, release, spending, or destructive operations requiring new authority.

Bundle up to three related Red decisions into one packet: decision, why Red, recommendation,
alternatives, consequences, and independent work that continues while awaiting the answer.
Do not ask separately for direct consequences of an already approved principle.

## 5. Three-vendor independent review

Every required cross-vendor gate uses the same artifact/content hash and a review brief that
states all primary roles:

- **Codex**: executable reality checks, boundary probes, concurrency, atomicity, data loss,
  and tests.
- **Claude**: canon/responsibility alignment, semantic consistency, propagation, and missing
  contracts. Static read-only.
- **Grok**: adversarial behavior, user journey, distribution, operations, recovery, and
  relevant external patterns. Static read-only with web disabled unless explicitly authorized.

Primary roles reduce duplicated full scans; they do not forbid cross-role findings.
Every vendor must also perform the common safety core: Red classification, secrets/auth,
data-loss/custody, external effects, artifact/hash provenance, and settlement integrity; then
perform its specialty deep dive and a residual C/I scan outside that specialty.

Normal review defaults: Codex `gpt-5.6-sol/high`, Claude `fable/high`, Grok
`grok-4.5/high`. Auth, data loss, custody, restore, difficult concurrency, and workflow/harness
changes require the strongest pinned model and `max` effort for all three; Claude uses `opus`.
Confirmation rounds use `high` after a strongest-matrix discovery round when every C/I has a
bounded prescription and executable negative probes cover the changed guards. Auth/custody/
restore product-contract settlement retains the strongest matrix through the final round.

Dispatch all required vendors in parallel through:

```bash
REVIEW_MANIFEST=<manifest> CODEX_EFFORT=high scripts/codex.sh review <prompt> <label>
REVIEW_MANIFEST=<manifest> CLAUDE_REVIEW_MODEL=fable CLAUDE_REVIEW_EFFORT=high scripts/claude-review.sh <prompt> <label>
REVIEW_MANIFEST=<manifest> GROK_REVIEW_EFFORT=high scripts/grok-review.sh <prompt> <label>
```

### Settlement

- All required vendors review the same content hash.
- Any Critical/Important is triaged, even when raised by only one vendor.
- Fix or reject a C/I autonomously when settled authority determines the answer; only Red
  semantic changes go to the user.
- Confirmation goes to each owner of a fixed/rejected C/I. Exact transcription may reduce an
  intermediate confirmation round, but any content change requires a final all-required-vendor
  round on the final hash.
- Semantic changes beyond a prescription return to every affected vendor.
- Minor-only findings do not block; fix or ledger them.
- `SETTLED` means zero unresolved C/I from all required vendors on the final hash.

## 6. Review consumption and parallel work

From dispatch until result, the reviewed artifact is frozen for reads and writes so line
anchors and evidence remain stable. Build a mode-aware manifest with
`scripts/review-manifest.sh`; pass it as `REVIEW_MANIFEST` so every atomic result receipt binds
vendor/model/effort, prompt hash, manifest hash, result hash, and timestamps. An `UNBOUND`
receipt cannot count toward settlement. Record manifest/prompt paths and hashes, vendors owed,
per-vendor state, result/receipt paths and verification in `docs/superpowers/active-ledger.md`.

The active ledger and generated receipts are operational state, not part of the substantive
artifact manifest they describe. The ledger records an artifact/base HEAD rather than claiming
a self-referential current commit.

The freeze is artifact-scoped. Continue work that does not consume the pending result:
reality checks, independent spikes, test infrastructure, evidence collection, and unrelated
artifacts. Missing ledger state never proves settlement; fail closed.

## 7. Plans and implementation

Plans constrain contracts rather than dictate implementations. Each task states:

- contract/invariants and forbidden scope;
- dependencies and first failing test;
- completion checks and verification commands;
- independent-review focus and commit boundary;
- rollback/interruption behavior.

Exact helper names and code snippets are fixed only where the public/semantic contract needs
them. Safer, simpler implementation discovered during work is Green if spec meaning is
unchanged.

Product implementation remains worker-driven through `scripts/codex.sh impl`; Main verifies
with `scripts/verify.sh`, runs three-vendor review, reconciles findings, and commits one
intentional unit at a time. Final integration review remains mandatory.

The independent-review iron law is absolute: an author/Main analysis is not its own independent
review. Read each applied fix hunk back against the prescription; workspace-wide absence claims
require the search scope/command and reviewer confirmation. If the same issue survives two fix
attempts, stop that loop and escalate it.

## 8. Persistent active ledger

`docs/superpowers/active-ledger.md` is the restart authority for workflow state, not product
design authority. It records real HEAD, phase, mission, settled hashes, in-flight reviews,
unresolved Red packets, Yellow decisions/triggers, next executable work, verification, and
user decisions. Git/disk/test override its factual claims when they disagree.

Update it before a long dispatch, before stopping, after a user decision, after a review
settles, and after each commit. Store the base/artifact HEAD, not a circular promise about the
commit containing the ledger update. Do not store secrets or large review bodies in it.

## 9. Stop conditions and external effects

Wait for the user only when a Red decision blocks the full critical path, canon cannot resolve
a contradiction, possible data loss/secret exposure/auth bypass cannot be excluded, required
independent review has failed, or new authority is needed for destructive/external action.

Commits within the approved mission are autonomous. Push, PR, merge, release, publication,
external messages, and spending remain separately authorized unless the user explicitly
included them in the mission.

## 10. Plan-6 trial and rollback

Plan 6 runs at **Yellow autonomy**: Green/Yellow proceed through commits; Red decisions are
bundled; the user receives milestone reports rather than section-by-section approvals.

Track human gates, Red/Yellow/Green counts, reversed classifications, review wall time,
duplicate findings, confirmation rounds, Design-Ready catches, and autonomous reverts.
Rollback to shadow autonomy if a Yellow decision is later found Red, a serious defect stems
from reduced review scope/effort, or speculative work causes material discard. The user makes
the post-plan evaluation.
