# Codex Eval Perspectives — Spec Review

Perspectives injected into Codex eval-review during **spec/design** phase.
Focus: architecture decisions, scope, abstraction boundaries, config strategy.

## Active Perspectives (max 10)

- **[2026-03-27]** `biased` in `tokio::select!` creates starvation risk when one channel has sustained traffic — applies to any fan-in or adapter loop. Learned from: rpi-local-adapter impl review. Review by: 2026-06-27
- **[2026-03-27]** Partial config surfaces (some env vars, some hardcoded) create an awkward middle ground that invites silent misconfiguration. Either fully hardcode or fully expose. Learned from: rpi-local-adapter gateway integration. Review by: 2026-06-27
- **[2026-03-28]** When a runtime claims "sensor-specific logic only / zero boilerplate," verify that transport-level metadata (ConnectionInfo, bus/address parameters) is constructed by the runtime, not repeated in each driver. Boilerplate that drifts across drivers undermines the abstraction. Learned from: polling-adapter-runtime config rename review. Review by: 2026-06-28
