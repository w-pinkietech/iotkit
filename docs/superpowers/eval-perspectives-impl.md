# Codex Eval Perspectives — Implementation Review

Perspectives injected into Codex eval-review during **implementation** phase.
Focus: code quality, error handling patterns, testing, runtime behavior.

## Active Perspectives (max 10)

- **[2026-03-27]** Every I2C error message must include bus path and address for field debugging on multi-bus/multi-sensor systems. Generic "read failed" messages are untriageable. Learned from: rpi-local-adapter sensor modules. Review by: 2026-06-27
- **[2026-03-27]** `let _ = channel.send()` in some branches but `if send().is_err() { return }` in others creates inconsistent shutdown behavior. All send paths in a loop must handle closed channels the same way. Learned from: rpi-local-adapter polling_loop. Review by: 2026-06-27
- **[2026-03-28]** Timestamps must be captured at the point of observation (e.g. successful I2C read), not at event construction time. Batch-collect-then-emit patterns (poll_cycle → apply_outcomes) can introduce seconds of skew on degraded buses. Applies to any adapter that collects multiple readings before emitting events. Learned from: timestamp-provenance design review. Review by: 2026-06-28
