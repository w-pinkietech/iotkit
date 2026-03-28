---
name: agent-team
description: Use after codex-eval-plan passes to dispatch implementation as a background agent team. Main agent spawns a Lead agent which manages Dev and Reviewer subagents. Main agent is freed for user interaction.
---

# Agent Team

Dispatch implementation as a background agent team. Main agent is freed for user interaction.

## When to Use

- After `codex-eval-plan` passes (zero Critical/Important)
- Plan and spec are committed and available in the repo

## Architecture

```
Main agent (user dialogue)
  └─ spawn → Lead agent (background, manages everything)
       ├─ per task:
       │   ├─ spawn → Dev subagent (worktree, implements + tests + commits)
       │   ├─ spawn → Spec reviewer subagent (codex-eval-impl-spec)
       │   │   FAIL → Dev fixes → re-review
       │   ├─ spawn → Quality reviewer subagent (codex-eval-impl-quality)
       │   │   FAIL → Dev fixes → re-review (quality only)
       │   └─ merge to feature branch
       ├─ final codex-eval (full diff)
       └─ report back to Main agent
```

**Main agent MUST NOT implement code.** Main agent only:
1. Spawns the Lead agent with `run_in_background: true`
2. Communicates Lead's result to the user
3. Creates PR if user approves

## How to Dispatch

Main agent spawns Lead with a single Agent call:

```
Agent tool:
  description: "Lead: implement [feature name]"
  run_in_background: true
  prompt: |
    {Lead agent prompt — see template below}
```

## Lead Agent Prompt Template

```
You are the Lead agent for implementing [feature name].

## Your Role

You manage Dev and Reviewer subagents. You do NOT write code yourself.
Your job: dispatch, coordinate, judge, merge.

## Plan

[FULL TEXT of the plan file — paste it, don't make Lead read it]

## Spec

[FULL TEXT of the spec file — paste it, don't make Lead read it]

## Working Branch

Create a feature branch from master: `feature/[feature-name]`

## Per-Task Flow

For each task in the plan, in order:

### 1. Dispatch Dev subagent

Use the Agent tool with isolation: "worktree":

  description: "Dev: Task N [name]"
  isolation: worktree
  prompt: |
    {Dev subagent prompt — see template below}

Wait for Dev to complete. Check status:
- DONE → proceed to review
- DONE_WITH_CONCERNS → read concerns, decide if blocking
- NEEDS_CONTEXT → provide context, re-dispatch
- BLOCKED → assess and handle (more context, different model, or break task)

### 2. Dispatch Spec reviewer subagent

  description: "Spec review: Task N"
  prompt: |
    You are a spec compliance reviewer using codex-eval-impl-spec.

    Read the skill at .claude/skills/codex-eval-impl-spec/SKILL.md
    Read the review guide at docs/eval/impl-spec-review.md

    ## Task Spec
    [task text from plan]

    ## Dev Report
    [what Dev reported]

    Run `codex exec` following codex-eval-common rules.
    Iterate until zero Critical/Important.
    Report: PASS or FAIL with specific issues.

If FAIL: send issues to Dev (via SendMessage), Dev fixes, re-review.

### 3. Dispatch Quality reviewer subagent

Only after spec review PASSES:

  description: "Quality review: Task N"
  prompt: |
    You are a code quality reviewer using codex-eval-impl-quality.

    Read the skill at .claude/skills/codex-eval-impl-quality/SKILL.md
    Read the review guide at docs/eval/impl-quality-review.md

    ## Dev Report
    [what Dev reported]

    Run `codex exec` following codex-eval-common rules.
    Iterate until zero Critical/Important.
    Report: PASS or FAIL with specific issues.

If FAIL: send issues to Dev, Dev fixes, re-review (quality only).

### 4. Merge

After both reviews PASS:
- Merge Dev's worktree changes to feature branch
- Commit
- Move to next task

## After All Tasks

1. Run final Codex evaluation on the full diff (feature branch vs master)
   Focus: cross-task consistency, integration issues
2. Run `cargo test --workspace` to verify everything passes
3. Report back to Main agent with:
   - Summary of what was implemented
   - All tasks completed / any issues
   - Test results
   - Ready for PR: yes/no

## Rules

- NEVER write implementation code yourself
- NEVER skip reviews (both stages required for every task)
- NEVER proceed to next task while current task has open review issues
- If a Dev subagent is BLOCKED twice on the same task, escalate to Main
- Use the cheapest model that can handle each role (haiku for mechanical tasks, sonnet for integration)
```

## Dev Subagent Prompt Template

```
You are implementing Task N: [task name]

## Task Description

[FULL TEXT of task from plan]

## Context

[Where this fits, what was done in previous tasks, dependencies]

## Before You Begin

If anything is unclear about requirements, approach, or dependencies — ask now.

## Your Job

1. Implement exactly what the task specifies
2. Write tests following TDD
3. Run tests: `cargo test -p [crate] && cargo test --workspace`
4. Commit your work
5. Self-review: completeness, quality, discipline, testing
6. Report back

## Self-Review Checklist

- Did I implement everything in the task spec?
- Did I add anything not in the spec? (remove it)
- Do type names, field names match the plan exactly?
- Are tests verifying behavior, not just compilation?
- Are error messages including sufficient context?

## Report Format

- **Status:** DONE | DONE_WITH_CONCERNS | BLOCKED | NEEDS_CONTEXT
- What you implemented
- Test results
- Files changed
- Self-review findings
- Any issues or concerns
```

## Integration with superpowers

This skill replaces `superpowers:subagent-driven-development` for this project.
Key differences:
- Main agent is freed (Lead runs in background)
- 2-stage review uses Codex (codex-eval-impl-spec + codex-eval-impl-quality)
- Dev subagents do NOT run Codex themselves (reviewers handle it)
- Lead manages all coordination
