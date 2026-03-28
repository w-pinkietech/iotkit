# CLAUDE.md

## Project

`iotkit-next` — IoT gateway for Raspberry Pi. Rust + tokio async runtime.
Rebuilds legacy `iotkit` with clean core/adapter/driver separation.

## Workspace Structure

```
core/types/          — domain types (AdapterEvent, SensorReading, DeviceKey, etc.)
core/engine/         — in-memory device state projection (Engine, DeviceView)
bravepi-mainboard-adapter/ — BravePI UART streaming adapter
iotkit-polling-adapter-runtime/ — shared scaffolding for I2C polling adapters
rpi-local-adapter/   — RPi direct-attached I2C adapter (uses polling runtime)
rpi4b-driver/        — low-level RPi4B transport (I2C, serial)
iotkit-gateway/      — composition root (AdapterHost fan-in, main binary)
```

Dependency direction: `core/types` <- `core/engine` <- adapters <- `iotkit-gateway`.

## Build & Test

```bash
cargo test --workspace        # all tests
cargo test -p <crate-name>    # single crate
cargo check                   # compile check
```

## Development Workflow

### Role Separation (MANDATORY)

**Main agent** must NEVER write implementation code directly.

| Role | Responsibilities | Forbidden |
|---|---|---|
| Main agent | User dialogue, requirements gathering, spec writing, Codex eval dispatch, result communication | Edit/Write on source code, running cargo commands for implementation |
| Lead agent | Task management, dev subagent dispatch, merge coordination, final review | Direct user interaction |
| Dev subagent | Implement one task, run tests, run codex-eval-impl, commit | Work on multiple tasks, skip Codex eval |

### Full Pipeline

```
1. brainstorming → spec
2. codex-eval-spec → iterate until clean
3. writing-plans → plan
4. codex-eval-plan → iterate until clean
5. Main agent spawns Lead agent (background)
   └─ Lead agent per task:
      ├─ Spawn Dev subagent (worktree)
      ├─ Dev implements + tests + codex-eval-impl
      ├─ Lead merges result
      └─ Repeat for next task
   └─ Lead runs final codex-eval-impl on full diff
   └─ Lead reports back to Main agent
6. Main agent communicates result to user
7. PR creation
```

### Codex Evaluation (MANDATORY at every stage)

Every artifact must pass Codex (GPT-5.4) evaluation before proceeding:

| Stage | Skill | Evaluator runs on |
|---|---|---|
| After spec | `codex-eval-spec` | Main agent (background subagent) |
| After plan | `codex-eval-plan` | Main agent (background subagent) |
| After each task impl | `codex-eval-impl` | Dev subagent (runs `codex exec` itself) |
| After all tasks | `codex-eval-impl` | Lead agent |

Iterate until zero Critical/Important findings. See `codex-eval-common` for shared rules.

Dev subagents MUST include Codex evaluation status in their completion report.
Lead agent MUST verify dev subagents ran Codex eval.

### Codex CLI

```bash
codex exec -m gpt-5.4 -c reasoning_effort=xhigh -s read-only \
  -o /tmp/codex-eval-{phase}-{id}-iter{n}.txt \
  "prompt"
```

Always: `-s read-only`, unique output paths, fresh session per iteration.
Never: `--full-auto`, `-s workspace-write`.

## Review Assets

| Phase | Perspectives | Checklist |
|---|---|---|
| Spec | `docs/superpowers/eval-perspectives-spec.md` | `docs/architecture-review-checklist.md` |
| Plan | `docs/superpowers/eval-perspectives-plan.md` | `docs/plan-review-checklist.md` |
| Impl | `docs/superpowers/eval-perspectives-impl.md` | `docs/coding-review-checklist.md` |

## Code Conventions

- Rust 2021 edition, tokio async runtime
- `tracing` for structured logging (not `println!`)
- Errors must include context (bus path, address, device key) for field debugging
- Channel send consistency: all send paths in a loop must handle closed channels the same way
- `SystemTime` for wall-clock timestamps, `tokio::time::Instant` for relative timing
- Pattern matches must be exhaustive — use `..` only in tests, not production code

## Commit Style

```
feat(crate): short description
fix(crate): short description
refactor(crate): short description
chore(crate): short description
docs: short description
```

Co-author line required for AI-generated commits:
```
Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>
```
