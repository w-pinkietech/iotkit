---
type: Guide
title: "AGENTS index"
description: "Common entry index for coding agents: always-on rules and links to .agents guides."
language: en
translation_key: agents.index
status: stable
revision: 1
---

# AGENTS.md

Common repository guidance for coding agents and human maintainers.
Agent-specific files may point here but must not redefine these rules.

Detailed rules live under [`.agents/`](.agents/). Read this index first, then
only the linked files needed for the task.

Agent guides use an OKF-like YAML frontmatter (`type`, `title`, `description`,
`language`, `translation_key`, `status`, `revision`) so tools and humans can
search and filter them. They are **not** part of the `docs/okf/` product bundle
and are not checked by `scripts/check-okf-docs.mjs`. Profile notes:
[`.agents/README.md`](.agents/README.md).

## Always

1. **Issue-driven development** — every change maps to one GitHub issue with
   outcome and exclusions; work in `agent/issue-<n>-*` + worktree; draft PR
   closes the issue; merge only after explicit human approval.
   Details: [`.agents/workflow.md`](.agents/workflow.md).
2. **Product docs** — current human-readable corpus is `docs/okf/` (both
   languages together). Start at `docs/README.md`.
   Details: [`.agents/documentation-authority.md`](.agents/documentation-authority.md).
3. **Invariants (summary)** — no secrets in logs/issues/PRs; no silent data loss;
   mutations only via typed ops dispatchers; MQTT PUBACK is not Edge durable
   accept. Full list: [`.agents/product-invariants.md`](.agents/product-invariants.md).
4. **Lightest sufficient process** — Fast by default; focused tests before
   behavior changes. Lanes: [`.agents/workflow.md`](.agents/workflow.md).

## Index

| Topic | File |
|---|---|
| Product overview and data flow | [`.agents/project-overview.md`](.agents/project-overview.md) |
| Documentation authority (OKF / redesign / superpowers) | [`.agents/documentation-authority.md`](.agents/documentation-authority.md) |
| What to read before editing (change map) | [`.agents/change-map.md`](.agents/change-map.md) |
| Issue loop, worktree, PR, change lanes | [`.agents/workflow.md`](.agents/workflow.md) |
| Common commands | [`.agents/commands.md`](.agents/commands.md) |
| Source and test placement | [`.agents/source-and-tests.md`](.agents/source-and-tests.md) |
| Review and verification | [`.agents/review-and-verification.md`](.agents/review-and-verification.md) |
| Product invariants (full) | [`.agents/product-invariants.md`](.agents/product-invariants.md) |
| Battle-tested review skill | [`.agents/skills/iotkit-battle-tested-review/SKILL.md`](.agents/skills/iotkit-battle-tested-review/SKILL.md) |

## Start of a task

1. Confirm or open the **GitHub issue** (outcome, non-goals, lane).
2. Read **Always** above.
3. Open only the matching row in [`.agents/change-map.md`](.agents/change-map.md).
4. Follow [`.agents/workflow.md`](.agents/workflow.md) for branch/worktree/PR.
5. Use the smallest disproof from [`.agents/commands.md`](.agents/commands.md).
