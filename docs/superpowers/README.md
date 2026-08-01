# Superpowers development artifacts

The `specs/` and `plans/` directories hold optional process artifacts for
scoped development issues. A reviewed specification can guide an active issue;
an implementation plan can preserve ordered execution context. After the change
merges, both freeze as lineage for that completed effort.

These artifacts never override current product documentation, a versioned
contract unit, code, or executable tests. Lasting product facts belong in their
owning current source, not only in a specification or plan.

## Responsibilities

- The GitHub issue owns the outcome, non-goals, lane, acceptance evidence, and
  scope changes.
- `specs/` records reviewed design choices, boundaries, and verification
  strategy when an issue needs durable design context.
- `plans/` records ordered implementation and verification steps when sequencing
  needs durable context. A plan is not a product contract or backlog.
- The pull request owns the actual diff, verification evidence, review, and
  product-document impact judgment.

## Create only when needed

The change lanes in [`AGENTS.md`](../../AGENTS.md) remain the process authority:

- **Fast:** create neither artifact by default.
- **Standard:** create a specification for multiple credible approaches, a
  meaningful new internal boundary, or a real UX decision.
- **Full:** create a specification when design decisions need approval before
  implementation; put lasting product or contract decisions in their owning
  current authority in the same change.
- **Any lane:** create a plan only for several order-dependent tasks,
  irreversible steps, or work that needs durable context across sessions.

Do not create a specification merely to repeat an issue or existing authority.
Do not create a plan when a short issue checklist is sufficient.

## Active work to lineage

1. Link every new specification and plan to its issue.
2. Review and approve a needed specification before implementation.
3. Use a needed plan only for that issue's implementation and verification.
4. After merge, stop updating both artifacts. A later behavior change uses a
   new issue and, when needed, new artifacts.

When executing a multi-task plan with Codex subagents, Main should dispatch
**implementer → executor → reviewer** per task (not one agent owning all three).
See [`.codex/README.md`](../../.codex/README.md).

Keep this directory permanently for lineage. Do not bulk-delete, rename, move,
or rewrite old artifacts to resemble the current product. Existing files may
contain former Site vocabulary, superseded scope, unchecked steps, or
implementation state from a specific point in time. Those properties are
historical evidence, not current work instructions.

The policy is tracked by [#145](https://github.com/w-pinkietech/iotkit/issues/145)
and its approved
[artifact lifecycle design](specs/2026-08-02-superpowers-artifact-lifecycle-design.md).
The completed historical spec-gap survey remains on
[#143](https://github.com/w-pinkietech/iotkit/issues/143); absorbing old
Superpowers material into product docs remains secondary to actual current
product gaps.
