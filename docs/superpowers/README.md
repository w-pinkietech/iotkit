# ⚠️ Historical execution records — not current guidance

Everything under `docs/superpowers/` (specs and plans alike) is the **execution
record of past work**, kept for archaeology. It is intentionally left as
written, so it contains instructions that were correct THEN and are wrong NOW.
Known examples: the Wave-0 plan 1 tells you to set SQLite `synchronous=NORMAL`
(superseded by D8 波及修正4 — custody-critical transactions MUST be `FULL`,
implemented 2026-07-10), and the 2026-03 command-boundary design teaches
extending `AdapterCommand` — the exact move the D4/D12 vocabulary freeze now
forbids (southbound verbs go to the future `iotkit-southbound-contract`).

Current guidance lives in exactly two places:

- **What/why**: the design corpus `../../../docs/redesign/` (D1–D13, R1–R23)
- **Where code goes / how it's structured**: [`docs/architecture.md`](../architecture.md)

The one file here that IS live: `plans/wave1-plan5-deferred-hardening.md` and
its successors — the deferred-item ledgers that upcoming plans must sweep.
