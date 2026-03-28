# Codex Eval Perspectives

## Active Perspectives (max 10)

- **[2026-03-27]** `biased` in `tokio::select!` creates starvation risk when one channel has sustained traffic — applies to any fan-in or adapter loop. Learned from: rpi-local-adapter impl review. Review by: 2026-06-27
- **[2026-03-27]** Partial config surfaces (some env vars, some hardcoded) create an awkward middle ground that invites silent misconfiguration. Either fully hardcode or fully expose. Learned from: rpi-local-adapter gateway integration. Review by: 2026-06-27
- **[2026-03-27]** Every I2C error message must include bus path and address for field debugging on multi-bus/multi-sensor systems. Generic "read failed" messages are untriageable. Learned from: rpi-local-adapter sensor modules. Review by: 2026-06-27
- **[2026-03-27]** `let _ = channel.send()` in some branches but `if send().is_err() { return }` in others creates inconsistent shutdown behavior. All send paths in a loop must handle closed channels the same way. Learned from: rpi-local-adapter polling_loop. Review by: 2026-06-27
- **[2026-03-28]** When a runtime claims "sensor-specific logic only / zero boilerplate," verify that transport-level metadata (ConnectionInfo, bus/address parameters) is constructed by the runtime, not repeated in each driver. Boilerplate that drifts across drivers undermines the abstraction. Learned from: polling-adapter-runtime config rename review. Review by: 2026-06-28
- **[2026-03-28]** Timestamps must be captured at the point of observation (e.g. successful I2C read), not at event construction time. Batch-collect-then-emit patterns (poll_cycle → apply_outcomes) can introduce seconds of skew on degraded buses. Applies to any adapter that collects multiple readings before emitting events. Learned from: timestamp-provenance design review. Review by: 2026-06-28
