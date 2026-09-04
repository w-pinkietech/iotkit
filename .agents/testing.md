# Testing policy

Acceptance evidence is the end-to-end **journey** (Japanese: 一気通貫テスト):
one script that starts the product, feeds it input, and checks the result at
the consumer. Unit tests exist only where a journey cannot practically reach.
Decided in [#233](https://github.com/w-pinkietech/iotkit/issues/233) for the
redesign in [#232](https://github.com/w-pinkietech/iotkit/issues/232).

## Journey stages

| Stage | Setup | What it proves | Where it runs |
|---|---|---|---|
| L1 minimal loop | `trial-sample` → IoTKit → Mosquitto → independent consumer | Published topic and payload bytes match the fixtures under `testdata/observation/v1`; the accumulated count matches the value computed from the sample waveform; heartbeats arrive | CI (docker compose) |
| L2 fault injection | L1 plus Broker stop/start, `kill -9` and restart of IoTKit, a threshold change from the Console | The outbox converges to PUBACK-acknowledged; series and sequence stay continuous; the threshold change applies without restart and keeps the series; Will and the graceful `offline` arrive | CI, second half of the L1 script |
| L3 Pinkiet | IoTKit (or the Pinkiet-side simulator) → Mosquitto → Pinkiet | A pre-registered Work Center shows the count and state on Gantt and Andon; `degraded` and pipeline deletion appear as unavailable input | Pinkiet repository |
| L4 real device | BravePI or rpi-local with a real sensor → IoTKit → Mosquitto → Pinkiet | A real machine is measured and its process is visible | Manual; procedure in the installation document, evidence on the issue |

L1 and L2 live in one script. Expected values come from the deterministic
`trial-sample` waveform; every wait is a bounded condition wait, never a fixed
sleep. The contract bridge to L3 is the fixture set: Pinkiet's consumer
conformance test and its simulator generate payloads from these fixtures, not
by hand.

## Unit tests that stay

- Evaluator semantics: hysteresis, debounce boundary times, calibration. Timing
  branches are slow and flaky to enumerate through a journey.
- Series start decision: the normalized hash of structural pipeline fields.
  Distinguishing structural from tuning changes is an enumeration problem.
- Single-transaction persistence: evaluation state, accumulated value,
  sequence, and outbox insert commit or roll back together. Crash boundaries
  are easier to reproduce in isolation.

Do not write unit tests for TOML parsing, typed operations, Console rendering,
or MQTT client details. L1 and L2 cover them.

## While the journey cannot run yet

The journey needs the MQTT Output Adapter from #232 child issue 4. Until then
`master` must compile, pass the unit tests that stay, and pass the
documentation checks. The gates change per child issue:

| Stage | Required |
|---|---|
| #233 (this policy) | lightweight checks, full Rust workspace, `human approval`, CodeQL |
| #232 child 1 (contract) | plus: fixtures validate against the JSON Schema; fixtures published with `mosquitto_pub` are matched by the consumer-side script |
| #232 children 2 and 3 | same |
| #232 child 4 (MQTT Output Adapter) | plus: L1 and L2 journey. Never removed after this point |
| #232 children 5 and 6 | same; `cargo test` shrinks as old crates are deleted |

CI has no changed-path selection. Every PR runs the lightweight lane and the
full Rust lane.

## Old integration scripts

`scripts/test-edge-*.sh`, `scripts/test-rust-edge-*.sh`,
`scripts/test-mqtt-security.sh`, `scripts/test-broker-cert*.sh`,
`scripts/test-certificate-hostname.sh`, and `scripts/test-iotkit-trial.sh`
verify the current product. CI no longer runs them. They are deleted together
with the documents that reference them in #232 child issue 5.

Return to [`AGENTS.md`](../AGENTS.md).
