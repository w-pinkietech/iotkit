# Development Workflow

Status: **Authoritative** (2026-07-11)

This document is the single workflow authority for `iotkit-next`. Design authority is
`docs/redesign/`; structural authority remains `docs/architecture.md`. `CLAUDE.md`,
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
| Medium | config, non-destructive migration, internal API, local reversible feature | short design note → Design Ready → independent Codex review → concise plan → implementation → integration review |
| Large / Red | auth, secrets, wire/API, custody, restore, threat model, major responsibility boundary | Mission Brief → Design Ready → spec → independent Codex review → contract-centered plan → plan review → task loops → final integration review |

Small requires at least one independent vendor; Main's own analysis never substitutes.
Medium and Large require a fresh read-only Codex review session. A targeted review that discovers contract or
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

## 5. Independent review (temporary degraded mode)

Cross-vendor review is temporarily unavailable because Claude subscription access is disabled
and Grok quota is exhausted. Every required gate uses a fresh read-only Codex session on the
same artifact/content hash. This is independent-session review, not cross-vendor assurance.

- **Codex**: executable reality checks, boundary probes, concurrency, atomicity, data loss,
  tests, and adversarial runtime behavior.
- **Codex also covers while degraded**: canon/semantic consistency, user journey, distribution,
  operations, recovery, and the mandatory common safety core.
- **Claude/Grok (optional)**: use only when service/quota is available. They create no debt
  unless explicitly opted in before dispatch.

Primary roles reduce duplicated full scans; they do not forbid cross-role findings.
Every vendor must also perform the common safety core: Red classification, secrets/auth,
data-loss/custody, external effects, artifact/hash provenance, and settlement integrity; then
perform its specialty deep dive and a residual C/I scan outside that specialty.

Model and effort follow the task, using the lowest effort that reliably produces the required
result:

| Work | Default | Escalation |
|---|---|---|
| Clear, repeatable, high-volume mechanical work | `gpt-5.6-luna/low` | Move to Terra if judgment or non-local tool use appears |
| Everyday settled-spec implementation | `gpt-5.6-terra/medium` | Move to Sol when the task becomes ambiguous, high-value, or contract-sensitive |
| Normal independent review | `gpt-5.6-sol/medium` | Use `high` for difficult multi-step or high-risk review |
| Design, auth/secrets, data loss/custody, restore, difficult concurrency, or workflow/harness work | `gpt-5.6-sol/high` | Use `xhigh` only for especially difficult work with substantial tradeoffs |

Plan 6 and every Large/Red or design workflow remain `gpt-5.6-sol/high` through confirmation
and final settlement; their internal subtasks do not downgrade merely because a step looks
mechanical. The lighter rows apply only to separately classified non-high-risk work. `max` is
exceptional, explicit, and reserved for the hardest single-agent problems; it is never a
routine default. A separate fresh review session remains mandatory; Main analysis cannot
substitute. Non-high-risk confirmation rounds use their applicable row once every C/I has a
bounded prescription and executable negative probes cover the changed guards.

Dispatch all required vendors in parallel through:

```bash
REVIEW_MANIFEST=<manifest> scripts/codex.sh review <prompt> <label>
```

Optional when available: `scripts/claude-review.sh` and `scripts/grok-review.sh` with the same
`REVIEW_MANIFEST` and prompt.

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

Product implementation remains worker-driven through `scripts/codex.sh impl`; its ordinary
default is Terra/medium, while Plan 6 and other high-risk work explicitly set
`CODEX_MODEL=gpt-5.6-sol CODEX_EFFORT=high`. Main verifies
with `scripts/verify.sh`, runs the required independent Codex review, reconciles findings, and commits one
intentional unit at a time. Final integration review remains mandatory.

### Hybrid Codex Cloud candidate lane

`scripts/codex-cloud.sh` may dispatch implementation or advisory-review work to a configured Codex
Cloud environment. Cloud submission is an external action and requires the authority applicable
to that task. The wrapper snapshots the prompt/query and records that local HEAD matched the live
remote branch immediately before dispatch; Cloud cannot consume uncommitted local state. Because
the installed CLI submits a mutable branch and does not return the checked-out commit, this is
local intent evidence, not proof of Cloud's actual base. Automatic Cloud apply remains disabled.

Cloud work remains on a candidate branch. A task ID, URL, lifecycle status, best-of-N attempt, or
diff is not a review result and cannot satisfy settlement. The installed Cloud CLI does not expose
the final answer body for hashing, so wrapper-generated Cloud collect receipts are explicitly
`settlement_eligible=false`. Unless a separately reviewed export/binding mechanism exists, a fresh
local `scripts/codex.sh review` remains required before `SETTLED` or integration to `master`.

While the user's computer is unavailable, a Cloud task may serve as temporary Main for candidate
work and ledger updates. It may not claim settlement, integrate to `master`, push/open a PR, merge,
release, or spend extra attempts without applicable authority. It records branch/base/commit, task
ID/URL, verification, review debt, timing, and next work. Returning local Main re-verifies Git,
ledger, tests, and receipts. Full mechanics and environment setup are in
`docs/cloud-development.md`.

Reconciled Cloud work follows the ordinary order without exception: verify → manifest-bound
independent review → reconcile and final verify → commit → separately authorized push/merge.

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

Commits within the approved mission are autonomous. Cloud task submission/extra attempts, push,
PR, merge, release, publication, external messages, and spending remain separately authorized unless the user explicitly
included them in the mission.

## 10. Plan-6 trial and rollback

Plan 6 runs at **Yellow autonomy**: Green/Yellow proceed through commits; Red decisions are
bundled; the user receives milestone reports rather than section-by-section approvals.

Track human gates, Red/Yellow/Green counts, reversed classifications, review wall time,
duplicate findings, confirmation rounds, Design-Ready catches, and autonomous reverts.
Rollback to shadow autonomy if a Yellow decision is later found Red, a serious defect stems
from reduced review scope/effort, or speculative work causes material discard. The user makes
the post-plan evaluation.

For each task, capture start/end timestamps. Task time starts immediately before the first scoped
planning or dispatch action after the preceding task boundary and ends when the task commit
completes (or when a blocked/stopped decision is recorded). Use wrapper-receipt UTC timestamps for
model runs, the Git committer timestamp for commit completion, and an ISO-8601 Main timestamp for
unwrapped boundaries.

Report one non-overlapping critical-path timeline whose categories are planning/research,
implementation, verification, independent review, finding fixes/reverification, and
documentation/commit. Attribute each interval to the phase currently gating completion; a model
run therefore remains implementation/review/fix time even if Main performs incidental work while
waiting. These exclusive categories must sum to total elapsed time. Separately report any reliably
measured overlapping Main activity or tool/model wait as supplemental activity metrics; they do
not sum to the elapsed total. Record model/effort and retry/confirmation counts in the active
ledger. Estimates and incomplete timestamp coverage must be labelled explicitly.
