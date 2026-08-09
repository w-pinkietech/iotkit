# Codex project agents (Main orchestration)

Custom subagents live in [`.codex/agents/`](agents/). Session defaults are in
[`config.toml`](config.toml). Upstream shape:
[Codex subagents](https://learn.chatgpt.com/docs/agent-configuration/subagents).

These roles define the routine Codex development loop: Main **splits**
implementation, fresh verification, and independent review instead of one
session doing all three alone.

## Roles

| Role | File | Configured sandbox default | Owns | Does not own |
|---|---|---|---|---|
| **implementer** | [agents/implementer.toml](agents/implementer.toml) | workspace-write | Routine settled code + focused tests for one task | Architecture decisions; independent review; suite-wide “green” claims |
| **complex_implementer** | [agents/complex-implementer.toml](agents/complex-implementer.toml) | workspace-write | Settled context-heavy or higher-risk code + focused tests | Architecture decisions; independent review; suite-wide “green” claims |
| **reviewer** | [agents/reviewer.toml](agents/reviewer.toml) | read-only | Spec-compliance and/or quality findings | Applying fixes; replacing test runs |

The sandbox column records custom-agent defaults, not a hard enforcement
boundary. Codex reapplies live parent-turn sandbox and approval overrides when it
spawns a subagent. Main must preserve the behavioral split even when the
effective sandbox is broader.

Main keeps: issue/plan scope, architecture, product and trust decisions,
implementation-lane selection, fresh acceptance verification, final acceptance,
worktree/branch/PR lifecycle, merge only after human approval, and **dispatch
order**.

## Implementation routing

Use `implementer` (Luna / Max) by default when the settled specification largely
determines the result: bounded bug fixes, boilerplate, wiring, straightforward
features, mechanical refactors, and routine focused tests.

Use `complex_implementer` (Terra / Max) only when correctness materially depends
on context or judgment that the handoff cannot fully encode, such as subtle
concurrency, difficult debugging, security- or custody-sensitive paths, public
contract or migration work, broad refactors, or a larger realistic blast radius.
Terra resolves difficult implementation details inside a settled architecture;
Main still owns architecture and policy decisions.

One failed Luna attempt may demonstrate that Main misclassified the task. Main
must first correct the handoff using the observed failure, then may escalate to
Terra. Do not repeat an unchanged prompt, choose a lane by prestige, or silently
substitute another role, model, or reasoning level. If the required custom agent
is unavailable, stop that lane and report the limitation.

Before accepting delegated work, use native spawn/details metadata when exposed
to confirm the selected role and its configured model/reasoning. If the runtime
does not expose a value, report it as unobserved rather than claiming verified
routing. A fresh task may be required after agent definitions change.

Implementation-agent checks are focused self-checks, not the acceptance gate.
Main inspects the actual diff and reruns the specified verification commands from
a fresh Main turn before acceptance. Reviewer findings are independent judgment;
they do not replace Main's command evidence.

Focused checks are the normal implementation evidence. Do not make
`scripts/verify.sh --workspace` a routine delegated-task sweep; it is an
explicit diagnosis. The `required CI` aggregate owns selected remote acceptance;
see the [verification ownership matrix](../.github/verification-ownership.md).

## Routine implementation loop

Use this loop for every routine task (or each clearly bounded plan/checklist
item), regardless of whether an optional process artifact is used.

```text
Main: settle task text, interfaces, constraints, and verification commands;
      select implementer or complex_implementer
  → selected implementation lane: implement + focused self-check only
  → Main: inspect the diff and rerun the required commands (fresh evidence)
  → reviewer: mode=spec-compliance against issue/spec/task
       if changes-requested → selected implementation lane → Main verification → reviewer (same mode)
  → reviewer: mode=quality (or one full pass if Main prefers a single review)
       if changes-requested → selected implementation lane → Main verification → reviewer
  → Main: final acceptance; mark task done; next task
After all tasks:
  → Main: broader verification warranted by risk
  → reviewer: mode=full on the whole branch/PR diff
  → Main: draft PR, product-doc impact, stop for human review
```

Any tracked diff change after a review invalidates that verdict. Run the relevant
Main verification commands again and obtain a fresh review before relying on
approval.

### Mapping from Superpowers skills

| Superpowers idea | Project role |
|---|---|
| Routine implementer subagent / task implementation | **implementer** |
| Context-heavy or higher-risk settled implementation | **complex_implementer** |
| Verification-before-completion / plan check commands | **Main** |
| Spec compliance reviewer | **reviewer** (`spec-compliance`) |
| Code quality / final code review | **reviewer** (`quality` or `full`) |
| Controller / orchestrator | **Main** (not a custom agent file) |

Do not dispatch multiple implementation agents (`implementer` or
`complex_implementer`) in parallel on the same worktree. Main and reviewer should
see implementation-agent output as untrusted until Main's fresh commands and
diff evidence support it.

For work outside a plan, use the same task-shaped implementation lane, Main
fresh verification, and independent reviewer sequence. For PR babysitting,
the reviewer reports findings, the selected implementation lane applies fixes,
and Main re-checks; no agent rewrites and self-approves.

## Handoff checklist (Main → subagent)

Always give:

1. Issue number and outcome / non-goals
2. Worktree path and base ref
3. Exact task objective and owned files or bounded responsibility (do not make
   the subagent re-read an entire plan file unless necessary; paste the task)
4. Interfaces that must remain compatible
5. Constraints, exclusions, and settled decisions
6. Exact verification commands and concrete success evidence
7. For **reviewer**: mode, BASE/HEAD SHAs or PR number, and the requirement
   source (issue/spec/plan task)

## Authority reminders

- Product law: `docs/product/` and paired contracts — not `docs/superpowers/`.
- Superpowers specs/plans: optional process artifacts; freeze after merge.
- Empty battle-tested or product-docs impact selection is not a safety proof.
- Human merge approval remains required.

See also [`.agents/workflow.md`](../.agents/workflow.md) and
[`.agents/review-and-verification.md`](../.agents/review-and-verification.md).
