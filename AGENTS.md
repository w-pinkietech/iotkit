# AGENTS.md

This is the common repository guidance for coding agents and human maintainers.
Agent-specific files may point here but must not redefine these rules.

Detailed guidance lives under [`.agents/`](.agents/). Read this index first,
then only the linked files needed for the task.

## Always

1. **Issue-driven development** — every development task maps to one GitHub
   issue with an outcome and exclusions. Work in the issue branch and worktree,
   open a PR that closes the issue, and merge only after explicit human
   approval. See [`.agents/workflow.md`](.agents/workflow.md).
2. **Current product authority** — start at [`docs/README.md`](docs/README.md).
   The human-readable product corpus is `docs/product/`; edit its `ja` and `en`
   files together. OKF v0.2 is the packaging format, not a second authority.
   See [`.agents/documentation-authority.md`](.agents/documentation-authority.md).
3. **Keep product docs current** — lasting product facts change in the same
   issue and PR as the behavior or contract. Temporary investigation stays on
   the issue or PR. Record updated product-doc paths or a concrete no-update
   reason in the PR. Use
   `node scripts/product-docs-impact.mjs select --base origin/master` as a
   lower-bound hint; empty output is not a safety proof. PR CI may soft-warn
   when candidates exist without docs/product updates or a no-update reason
   (never a merge blocker).
4. **Protect product invariants** — never expose secrets or customer data,
   never silently lose data, route mutations through typed operations, and do
   not confuse MQTT PUBACK with durable IoTKit acceptance. See
   [`.agents/product-invariants.md`](.agents/product-invariants.md).
5. **Use the lightest sufficient process** — Fast is the default. Add or update
   the closest focused test before changing product behavior, then widen
   verification only for realistic risk. See [`.agents/workflow.md`](.agents/workflow.md).

## Index

| Topic | File |
|---|---|
| Product overview and data flow | [`.agents/project-overview.md`](.agents/project-overview.md) |
| Documentation authority and freshness | [`.agents/documentation-authority.md`](.agents/documentation-authority.md) |
| What to read before editing | [`.agents/change-map.md`](.agents/change-map.md) |
| Issue loop, worktree, PR, and change lanes | [`.agents/workflow.md`](.agents/workflow.md) |
| Common verification commands | [`.agents/commands.md`](.agents/commands.md) |
| Source and test placement | [`.agents/source-and-tests.md`](.agents/source-and-tests.md) |
| Review and verification | [`.agents/review-and-verification.md`](.agents/review-and-verification.md) |
| Codex subagent roles (implementer / executor / reviewer) | [`.codex/README.md`](.codex/README.md) |
| Product invariants | [`.agents/product-invariants.md`](.agents/product-invariants.md) |
| Battle-tested review skill | [`.agents/skills/iotkit-battle-tested-review/SKILL.md`](.agents/skills/iotkit-battle-tested-review/SKILL.md) |

## Start of a development task

1. Confirm or open the GitHub issue and record the intended outcome,
   exclusions, lane, and acceptance evidence.
2. Read the **Always** rules above and [`docs/README.md`](docs/README.md).
3. Open only the matching row in [`.agents/change-map.md`](.agents/change-map.md).
4. Follow [`.agents/workflow.md`](.agents/workflow.md) for the branch,
   worktree, PR, product-doc impact, and merge loop.
5. Use the smallest disproof from [`.agents/commands.md`](.agents/commands.md).
