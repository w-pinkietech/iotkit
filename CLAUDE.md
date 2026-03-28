# CLAUDE.md

Rust + tokio IoT gateway for Raspberry Pi. `core/types` <- `core/engine` <- adapters <- `iotkit-gateway`.

## Build & Test

```bash
cargo test --workspace
cargo test -p <crate-name>
```

## Workflow Rules

- **Main agent は実装禁止。** 対話/spec/Codex eval dispatch のみ。実装は agent team (lead → dev subagent)。
- **Codex eval は全段階で必須。** spec → plan → per-task impl → final impl。dev subagent も自分で `codex exec` を実行する。
- Pipeline: brainstorming → codex-eval-spec → writing-plans → codex-eval-plan → agent team → codex-eval-impl → PR

## Reference Docs (read on demand)

| Topic | Path |
|---|---|
| Architecture review criteria | [docs/architecture-review-checklist.md](docs/architecture-review-checklist.md) |
| Coding review criteria | [docs/coding-review-checklist.md](docs/coding-review-checklist.md) |
| Plan review criteria | [docs/plan-review-checklist.md](docs/plan-review-checklist.md) |
| Eval perspectives (spec) | [docs/superpowers/eval-perspectives-spec.md](docs/superpowers/eval-perspectives-spec.md) |
| Eval perspectives (plan) | [docs/superpowers/eval-perspectives-plan.md](docs/superpowers/eval-perspectives-plan.md) |
| Eval perspectives (impl) | [docs/superpowers/eval-perspectives-impl.md](docs/superpowers/eval-perspectives-impl.md) |
| Codex review history | [codex-review.md](codex-review.md) |
| Agent instructions | [AGENTS.md](AGENTS.md) |

## Commit Style

`feat(crate):` / `fix(crate):` / `refactor(crate):` / `docs:` + Co-Authored-By line.
