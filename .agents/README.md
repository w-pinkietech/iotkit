---
type: Guide
title: "Agent guidance bundle profile"
description: "Layout and OKF-like frontmatter for AGENTS.md-linked guides under .agents/."
language: en
translation_key: agents.bundle
status: stable
revision: 1
---

# Agent guidance under `.agents/`

These files are the detailed half of [`AGENTS.md`](../AGENTS.md). They are
**development process guidance**, not the product corpus in `docs/okf/`.

## Frontmatter profile

Each guide (and `AGENTS.md`) starts with a small YAML block aligned with the
IoTKit OKF scalar style so search and inventory stay easy:

| Field | Meaning |
|---|---|
| `type` | Always `Guide` for agent process docs |
| `title` | Short human title |
| `description` | One-line summary (searchable) |
| `language` | `en` (process docs are English-first) |
| `translation_key` | Stable id, e.g. `agents.workflow` |
| `status` | `draft` · `stable` · `deprecated` |
| `revision` | Positive integer; bump when meaning changes |

Example:

```yaml
---
type: Guide
title: "Issue-driven development workflow"
description: "Issue first, worktree, draft PR, change lanes, and lightweight design style."
language: en
translation_key: agents.workflow
status: stable
revision: 1
---
```

Do **not** put product contracts or operator runbooks here—use `docs/okf/`.
Do **not** require bilingual pairs for `.agents/` guides unless a guide is later
promoted into OKF.

## Files

| `translation_key` | Path |
|---|---|
| `agents.index` | [`../AGENTS.md`](../AGENTS.md) |
| `agents.project-overview` | [`project-overview.md`](project-overview.md) |
| `agents.documentation-authority` | [`documentation-authority.md`](documentation-authority.md) |
| `agents.change-map` | [`change-map.md`](change-map.md) |
| `agents.workflow` | [`workflow.md`](workflow.md) |
| `agents.commands` | [`commands.md`](commands.md) |
| `agents.source-and-tests` | [`source-and-tests.md`](source-and-tests.md) |
| `agents.review-and-verification` | [`review-and-verification.md`](review-and-verification.md) |
| `agents.product-invariants` | [`product-invariants.md`](product-invariants.md) |
| `agents.bundle` | this file |

Skills remain under [`.agents/skills/`](skills/).
