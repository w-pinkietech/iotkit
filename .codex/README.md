# Codex project agents (Main orchestration)

Custom subagents live in [`.codex/agents/`](agents/). Session defaults are in
[`config.toml`](config.toml). Upstream shape:
[Codex subagents](https://learn.chatgpt.com/docs/agent-configuration/subagents).

These roles exist so Main (and Superpowers-style plan execution) can **split**
work instead of one session doing implement + verify + review alone.

## Roles

| Role | File | Configured sandbox default | Owns | Does not own |
|---|---|---|---|---|
| **implementer** | [agents/implementer.toml](agents/implementer.toml) | workspace-write | Settled code + focused tests for one task | Independent review; suite-wide “green” claims |
| **executor** | [agents/executor.toml](agents/executor.toml) | workspace-write | Fresh verification commands and evidence | Feature implementation; design opinions |
| **reviewer** | [agents/reviewer.toml](agents/reviewer.toml) | read-only | Spec-compliance and/or quality findings | Applying fixes; replacing test runs |

The sandbox column records custom-agent defaults, not a hard enforcement
boundary. Codex reapplies live parent-turn sandbox and approval overrides when it
spawns a subagent. Main must preserve the behavioral split even when the
effective sandbox is broader. Executor workspace-write is for prescribed setup
and build/test artifacts, not tracked-file fixes.

Main keeps: issue/plan scope, product and trust decisions, worktree/branch/PR
lifecycle, merge only after human approval, and **dispatch order**.

## When Superpowers (or a plan) is in use

Use this loop for **each** plan task (or each clearly bounded checklist item).
Fast single-file work may stay on Main alone; do not invent subagents for
noise.

```text
Main: settle task text, constraints, and verification commands
  → implementer: implement + focused self-check only
  → executor: run the plan’s / Main’s verification commands (fresh evidence)
  → reviewer: mode=spec-compliance against issue/spec/task
       if changes-requested → implementer → executor → reviewer (same mode)
  → reviewer: mode=quality (or one full pass if Main prefers a single review)
       if changes-requested → implementer → executor → reviewer
  → Main: mark task done; next task
After all tasks:
  → executor: broader verification warranted by risk
  → reviewer: mode=full on the whole branch/PR diff
  → Main: draft PR, product-doc impact, stop for human review
```

### Mapping from Superpowers skills

| Superpowers idea | Project role |
|---|---|
| Implementer subagent / task implementation | **implementer** |
| Verification-before-completion / plan check commands | **executor** |
| Spec compliance reviewer | **reviewer** (`spec-compliance`) |
| Code quality / final code review | **reviewer** (`quality` or `full`) |
| Controller / orchestrator | **Main** (not a custom agent file) |

Do not dispatch multiple **implementers** in parallel on the same worktree.
Executor and reviewer should see implementer output as untrusted until
commands and diff evidence support it.

## When Superpowers is not in use

Still split when useful:

- Main implements small Fast changes alone, then **executor** for the focused
  command set, then **reviewer** before asking a human to merge risky work.
- For PR babysitting: **reviewer** for findings, **implementer** for fixes,
  **executor** for re-check — not one agent rewriting and self-approving.

## Handoff checklist (Main → subagent)

Always give:

1. Issue number and outcome / non-goals
2. Worktree path and base ref
3. Exact task text or file list (do not make the subagent re-read an entire plan
   file unless necessary; paste the task)
4. For **executor**: full command list
5. For **reviewer**: mode, BASE/HEAD SHAs or PR number, and the requirement
   source (issue/spec/plan task)

## Authority reminders

- Product law: `docs/product/` and paired contracts — not `docs/superpowers/`.
- Superpowers specs/plans: optional process artifacts; freeze after merge.
- Empty battle-tested or product-docs impact selection is not a safety proof.
- Human merge approval remains required.

See also [`.agents/workflow.md`](../.agents/workflow.md) and
[`.agents/review-and-verification.md`](../.agents/review-and-verification.md).
