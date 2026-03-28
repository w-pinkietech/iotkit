# BaseAdapterConfig → PollingAdapterConfig Rename

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename `BaseAdapterConfig` to `PollingAdapterConfig` (and `to_base_config` → `to_polling_config`) to complete the terminology normalization started by the crate rename.

**Architecture:** Pure mechanical rename across 3 source files and 2 doc files. No logic changes. Hard cutover — no deprecation alias (all consumers are workspace-internal). `SensorTargetConfig` stays as-is — it's already transport-agnostic and accurate.

**Tech Stack:** Rust, cargo

---

## File Map

| File | Role | Change |
|------|------|--------|
| `iotkit-polling-adapter-runtime/src/lib.rs` | Struct definition + public API | Rename struct, doc comment, all usages |
| `iotkit-polling-adapter-runtime/src/polling_loop.rs` | Internal consumer | Update import + usages |
| `rpi-local-adapter/src/lib.rs` | Downstream consumer | Update import, function name, variable name, usages |
| `docs/superpowers/specs/2026-03-28-polling-adapter-runtime-design.md` | Design spec | Update all `BaseAdapterConfig` references |
| `docs/superpowers/plans/2026-03-28-polling-adapter-runtime.md` | Implementation plan | Update all `BaseAdapterConfig` references |

---

### Task 1: Rename Rust symbols (atomic — all 3 source files in one commit)

**Files:**
- Modify: `iotkit-polling-adapter-runtime/src/lib.rs`
- Modify: `iotkit-polling-adapter-runtime/src/polling_loop.rs`
- Modify: `rpi-local-adapter/src/lib.rs`

- [ ] **Step 1: Rename all `BaseAdapterConfig` → `PollingAdapterConfig` in lib.rs**

In `iotkit-polling-adapter-runtime/src/lib.rs`, use find-and-replace across the entire file:

- `BaseAdapterConfig` → `PollingAdapterConfig` (all occurrences — struct def, doc comments, function signatures, tests)
- Doc comment on struct: `/// Configuration for a base adapter instance.` → `/// Configuration for a polling adapter instance.`

Affected locations (use symbol search, not line numbers):
- Struct definition: `pub struct BaseAdapterConfig`
- Doc comment on `validate_config`: `` [`BaseAdapterConfig`] ``
- `validate_config` signature: `config: &BaseAdapterConfig`
- `start` signature: `config: BaseAdapterConfig`
- Test helper `stub_config` return type and body
- All test functions constructing `BaseAdapterConfig { ... }`

- [ ] **Step 2: Update polling_loop.rs**

In `iotkit-polling-adapter-runtime/src/polling_loop.rs`, replace all occurrences:

- `use crate::{BaseAdapterConfig,` → `use crate::{PollingAdapterConfig,`
- `config: BaseAdapterConfig` → `config: PollingAdapterConfig` (in struct field and function params)
- In test module: `use crate::{BaseAdapterConfig,` → `use crate::{PollingAdapterConfig,`
- `fn make_config(...) -> BaseAdapterConfig` → `fn make_config(...) -> PollingAdapterConfig`
- `BaseAdapterConfig {` → `PollingAdapterConfig {` (in test helper body)

- [ ] **Step 3: Update rpi-local-adapter/src/lib.rs**

Replace all occurrences:

- `use iotkit_polling_adapter_runtime::{BaseAdapterConfig,` → `use iotkit_polling_adapter_runtime::{PollingAdapterConfig,`
- `fn to_base_config(config: &RpiLocalConfig) -> BaseAdapterConfig` → `fn to_polling_config(config: &RpiLocalConfig) -> PollingAdapterConfig`
- `BaseAdapterConfig {` → `PollingAdapterConfig {` (in function body)
- `let base_config = to_base_config(&config);` → `let polling_config = to_polling_config(&config);`
- `base_config)` → `polling_config)` (passed to `start()`)

- [ ] **Step 4: Verify workspace compiles**

Run: `cargo check --workspace`
Expected: no errors

- [ ] **Step 5: Run all workspace tests**

Run: `cargo test --workspace 2>&1 | tail -10`
Expected: all tests pass across both crates

- [ ] **Step 6: Commit all Rust changes atomically**

```bash
git add iotkit-polling-adapter-runtime/src/lib.rs iotkit-polling-adapter-runtime/src/polling_loop.rs rpi-local-adapter/src/lib.rs
git commit -m "refactor: rename BaseAdapterConfig to PollingAdapterConfig"
```

---

### Task 2: Update documentation

**Files:**
- Modify: `docs/superpowers/specs/2026-03-28-polling-adapter-runtime-design.md`
- Modify: `docs/superpowers/plans/2026-03-28-polling-adapter-runtime.md`

- [ ] **Step 1: Update spec**

In `docs/superpowers/specs/2026-03-28-polling-adapter-runtime-design.md`, replace all occurrences:
- `BaseAdapterConfig` → `PollingAdapterConfig`
- `to_base_config` → `to_polling_config`
- `base_config` (as variable name in code snippets) → `polling_config`
- Section 4 heading: `## 4. BaseAdapterConfig` → `## 4. PollingAdapterConfig`

- [ ] **Step 2: Update implementation plan**

In `docs/superpowers/plans/2026-03-28-polling-adapter-runtime.md`, replace all occurrences:
- `BaseAdapterConfig` → `PollingAdapterConfig`
- `to_base_config` → `to_polling_config`
- `base_config` (as variable name in code snippets) → `polling_config`

- [ ] **Step 3: Verify no stale references in source and spec**

Run: `grep -rn 'BaseAdapterConfig\|to_base_config' --include='*.rs' iotkit-polling-adapter-runtime/ rpi-local-adapter/ docs/superpowers/specs/`
Expected: no output (zero matches)

Note: The rename plan file (`docs/superpowers/plans/2026-03-28-polling-config-rename.md`) intentionally contains the old names in its title/goal description and is excluded from this check.

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/specs/2026-03-28-polling-adapter-runtime-design.md docs/superpowers/plans/2026-03-28-polling-adapter-runtime.md
git commit -m "docs: update BaseAdapterConfig → PollingAdapterConfig in spec and plan"
```
