# Superpowers artifact lifecycle design

**Issue:** [#145](https://github.com/w-pinkietech/iotkit/issues/145)  
**Status:** Approved design  
**Lane:** Standard

## Goal

Allow IoTKit development to use Superpowers-style design specifications and
implementation plans when they provide real value, while keeping GitHub issues,
process artifacts, and current product authority as separate concerns.

The smallest observable loop is one scoped issue that can create a reviewed
specification or plan when its work needs one, implement and verify the change,
and leave current product facts in their owning authoritative artifacts after
merge.

## Non-goals

- Requiring a specification or plan for every issue.
- Making `docs/superpowers/` authoritative for current product behavior.
- Rewriting existing specifications or plans to match the current product.
- Moving, renaming, or bulk-deleting existing Superpowers artifacts.
- Adding CI, templates, status automation, or a new document-management system.
- Changing the treatment of `docs/redesign/`.

## Responsibilities

The artifacts have distinct responsibilities:

| Artifact | Responsibility |
|---|---|
| GitHub issue | Outcome, non-goals, lane, acceptance evidence, and scope changes |
| `docs/superpowers/specs/` | Reviewed design choices, boundaries, and verification strategy for one development effort |
| `docs/superpowers/plans/` | Ordered implementation and verification steps when sequencing needs durable context |
| Pull request | Actual diff, verification evidence, review, and product-document impact |
| `docs/product/`, paired contract artifacts, code, and tests | Current product authority after merge |

A specification or plan supports an issue; it does not replace the issue. A
process artifact can guide its development effort without becoming a product
contract.

## Creation policy

Creation is based on need, with the change lane supplying the default:

- **Fast:** create neither artifact by default.
- **Standard:** create a specification when the work has multiple credible
  approaches, introduces a meaningful internal boundary, or requires a real UX
  decision.
- **Full:** create a specification when design decisions must be approved before
  implementation. Record lasting product or contract decisions in the owning
  current authority in the same change.
- **Any lane:** create an implementation plan only when work contains several
  order-dependent tasks, irreversible steps, or enough work to require durable
  context across sessions.

Do not create a specification merely to restate an issue or existing authority.
Do not create a plan when a short issue checklist is sufficient.

## Lifecycle

1. Create or reuse the GitHub issue and record its outcome, non-goals, lane, and
   acceptance evidence.
2. If the creation policy selects a specification, write it under
   `docs/superpowers/specs/`, review it, and obtain approval before implementation.
3. If the approved design needs a durable ordered plan, write it under
   `docs/superpowers/plans/` before executing those steps.
4. Implement and verify in the issue worktree and branch. The pull request links
   the issue and any created process artifacts.
5. Before merge, put every lasting product fact in its owning `docs/product/`
   pair, contract artifact, code, or executable test.
6. After merge, stop updating the specification and plan. They become lineage
   for that completed development effort. A later behavioral change uses a new
   issue and, when needed, new artifacts.

Unchecked steps in a completed or superseded plan describe its historical
execution state; they are not automatically current backlog.

## Authority and conflict handling

`docs/superpowers/` is authoritative only for the approved process context of
its linked development effort while that effort is active. It never overrides
current product documentation, a versioned contract unit, code, or executable
tests.

When an active process artifact conflicts with current authority, implementation
stops until the issue resolves the conflict. When an old artifact conflicts with
current authority, the current authority wins and the old artifact remains
unchanged as historical evidence.

If an old artifact exposes a still-valid product gap, express that fact using
current terminology in the owning product or contract artifact; do not rewrite
the old evidence to appear current.

## Required repository changes

The implementation updates only the process and documentation-boundary text:

- `docs/superpowers/README.md` describes the active-to-lineage lifecycle.
- `.agents/workflow.md` replaces the blanket creation ban with the creation
  policy above.
- `.agents/documentation-authority.md` describes Superpowers artifacts as
  non-product-authoritative development artifacts that freeze into lineage.
- `docs/README.md` uses the same role in its documentation map.

No product document changes are required because this design changes repository
development process, not operator-, integrator-, or contract-visible behavior.

## Verification

- Read the four changed policy locations together and confirm that their
  authority, creation, and lifecycle statements agree.
- Search for obsolete blanket statements that prohibit every new file under
  `docs/superpowers/`.
- Run `node scripts/product-docs-impact.mjs select --base origin/master` and
  record the product-document impact judgment.
- Run `node scripts/check-product-docs.mjs`.
- Inspect the final diff to confirm that existing specifications and plans were
  not rewritten.
