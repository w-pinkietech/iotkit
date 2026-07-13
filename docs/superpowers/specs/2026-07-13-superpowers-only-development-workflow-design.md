# Superpowers-only development workflow

Status: **User-approved design** (2026-07-13)

## Goal

Return `iotkit-next` to the standard Superpowers development cycle and remove the project-specific
review bureaucracy that duplicated it. Development should spend time on design, implementation, and
risk-proportionate verification rather than manifests, receipts, settlement bookkeeping, repeated
confirmation reviews, or maintaining a second skill system.

The resulting flow is:

```text
brainstorming
  -> written design + user review
  -> writing-plans
  -> TDD implementation (directly or through the configured implementer role)
  -> code review at Superpowers checkpoints
  -> verification-before-completion
  -> finishing-a-development-branch
```

## Keep

- Superpowers skills as the only development-process framework.
- Product authority in `docs/redesign/` and structural authority in `docs/architecture.md`.
- The IoTKit invariants: never expose secrets, never silently lose data, and route mutations through
  R14 dispatch.
- Verification economy: checks are proportional to changed behavior and realistic failure paths;
  broad Rust verification remains required when relevant impact cannot be excluded.
- Project-scoped native role routing: Sol/high for Main and reviewer; Luna/max for implementer and
  executor. Role selection helps execution but creates no special settlement state.
- Normal Codex authority boundaries for destructive actions and external publication such as push,
  PR, merge, release, and spending.
- Optional Codex Cloud helpers as a separate operator tool. They are not part of the default local
  development pipeline or `scripts/verify.sh`.
- Historical product specs and implementation plans when they remain useful archaeology. Historical
  references to the removed workflow do not become active instructions.

## Delete

### Workflow authority and live bookkeeping

- `docs/development-workflow.md`.
- `docs/superpowers/active-ledger.md`.
- Obsolete live workflow evidence such as `docs/superpowers/PLAN6-DESIGN-READY.md`.
- Main/worker rules that prohibit Main from writing Rust, require Green/Yellow/Red classification,
  or require a persistent ledger before ordinary work can continue.

### Hash-bound review and settlement machinery

- Review manifests, receipts, result/event hash binding, `SETTLED`, final-hash confirmation loops,
  mandatory C/I-zero reruns, and review-hash commit trailers.
- `scripts/review-manifest.sh`, `scripts/review-receipt.sh`, and
  `scripts/check-codex-events.sh`.
- The mandatory review wrappers and their regression harness:
  `scripts/codex.sh`, `scripts/claude-review.sh`, `scripts/grok-review.sh`, and
  `scripts/test-codex.sh`.
- The over-specialized commit-trailer helper `scripts/trailer.sh`.

### Duplicate review framework

- `.claude/skills/codex-eval-common`, `codex-eval-spec`, `codex-eval-plan`,
  `codex-eval-impl`, `codex-impl-loop`, and `eval-perspectives-curator`.
- `docs/eval/`, `scripts/watchpoints.sh`, and `codex-review.md`.
- Model-role preflight scripts and their default verification cost:
  `scripts/check-codex-role-config.sh` and `scripts/test-codex-role-config.sh`.

## Update

- Reduce `AGENTS.md` and `CLAUDE.md` to project context, product invariants, verification economy,
  model-role intent, and a direct instruction to use applicable Superpowers skills.
- Reduce `scripts/verify.sh` to product checks: formatting, layer rules, workspace tests, and Clippy.
  Remove review-, receipt-, Cloud-, and model-routing harness checks from the default product gate.
- Update `docs/superpowers/README.md` so specs/plans are historical records and Superpowers skills are
  the process authority.
- Remove obsolete workflow/ledger references from active entry points. Historical specs and plans do
  not need mechanical rewriting unless they are still presented as current guidance.
- Mark the current minimum Gateway + Site Server design as user-approved and awaiting written-spec
  review, not independent-hash settlement.

## Review and verification after migration

Superpowers review remains available and useful, but it follows the skill checkpoints instead of a
project-specific proof protocol:

- `requesting-code-review` after a meaningful task, major feature, or before merge;
- `receiving-code-review` before acting on feedback;
- `verification-before-completion` before claiming success;
- no manifest, receipt, settlement label, or compulsory rereview merely because prose changed.

The workflow cleanup changes documentation and shell tooling, not Rust product behavior. Verify it
with focused checks:

- `git diff --check`;
- `bash -n scripts/verify.sh` and any retained changed shell script;
- `scripts/check-layers` to ensure the retained product boundary checker still runs;
- repository searches proving active entry points no longer reference deleted files or states.

Do not run the full Rust workspace suite solely for this deletion: it cannot exercise removed review
bookkeeping and the Rust sources are unchanged. The next Rust implementation milestone will run the
product checks appropriate to its impact.

## Migration and rollback

Perform the cleanup as one intentional workflow commit after this design is reviewed. Existing
uncommitted Gateway + Site Server design work must be preserved and not accidentally staged with the
workflow-design commit. The cleanup commit may update that active spec only where necessary to remove
the obsolete review status.

Rollback is an ordinary revert of the cleanup commit. No product schema, credential, stored data, or
external service is changed.
