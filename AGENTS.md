# AGENTS.md

## Project Context

`iotkit-next` is an in-progress remake of the legacy `iotkit` system.
The current goal is to rebuild legacy behavior incrementally while improving the separation of `core`, `adapter`, and `driver` layers.
When making changes, prefer preserving legacy-compatible behavior unless the change is an explicit architectural improvement or a deliberate deviation.

## Implementation Rules

実装前に必ず読むこと: [docs/eval/impl-rules.md](docs/eval/impl-rules.md)

思考原則（可逆変換、side-effect 前の状態変更禁止、並行ライフサイクル設計等）と library pitfalls をまとめている。
