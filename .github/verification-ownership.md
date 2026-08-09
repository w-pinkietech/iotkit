# Verification ownership matrix

This is the single ownership ledger for normal verification. `scripts/select-ci-jobs.mjs`
remains the only changed-path selector. “Owner” means the default place that
must run and report a suite; a developer may still run a focused check for fast
feedback. CI, not a reported local pass, is the merge authority.

`scripts/verify.sh --workspace` is an opt-in full-workspace diagnosis. It is
not a routine pull-request default and intentionally has no integration bucket.
Runtime labels are planning estimates, not timing guarantees. Command counts
are top-level default developer commands, not subprocesses or asynchronous CI
jobs.

The policy test discovers every `scripts/test-*.sh` file and requires exactly
one row below. “Release coverage” records deliberate calls from the host
release composite as `direct / nested`; it is extra evidence, not a second
default owner. `root` denotes the host release composite itself.

## Branch-protection assumption

Default-branch protection must require `required CI`, `human approval`, and
CodeQL. The trusted auto-merge workflow posts `human approval` as pending for
each opened, reopened, ready-for-review, or synchronized PR head, and posts
success only after a qualified exact comment. This change does not alter live
GitHub settings; Main enables the required context after this PR provides it
and receives approval.

## Default command comparison

| Scope | Before default commands | After default commands | Expected runtime / ownership |
| --- | ---: | ---: | --- |
| Routine Rust behavior | 7 | 2 | Before: focused check plus six-command workspace sweep, several minutes locally. After: focused test and lint, seconds to minutes; CI Rust owns merge evidence. |
| Former `verify.sh --full` integration bucket | 20 | 0 | Before: six workspace commands plus fourteen integration invocations, tens of minutes or more. After: no bucket; each suite has the owner below. |
| Opt-in workspace diagnosis | 6 | 6 | Several minutes; workspace diagnosis only when a maintainer explicitly requests cross-workspace diagnosis. |

## Representative diffs

| Diff | Before default commands | After default commands | Expected runtime / ownership |
| --- | ---: | ---: | --- |
| Docs only | 1 | 1 | Seconds locally; CI lightweight is the authoritative repository/documentation owner. |
| One Rust crate | 7 | 2 | Before duplicated a several-minute workspace sweep. After local focused feedback is seconds to minutes; CI Rust runs the selected package set. |
| Shared Rust core | 7 | 2 | Local focused feedback stays small; reverse dependencies safely expand CI Rust to the workspace when needed. |
| Console | 2 | 2 | Console checks and browser journey are minutes; CI Console is the default owner. |
| Custody or output | 8 | 2 | Local custody/output evidence is focused; CI Edge owns the selected Docker integration, usually minutes. |
| Trial | 1 | 1 | Trial journey is minutes; CI trial owns it when selected. |
| Unknown or CI infrastructure | 6 | 0 | No implicit local sweep; the selector fails closed to all CI lanes, so remote runtime is the full selected lane set. |

## Suite ownership

| ID | Suite / command | Owner | Trigger | Release coverage (direct / nested) | Expected runtime |
| --- | --- | --- | --- | --- | --- |
| required-ci | Stable `required CI` aggregate | CI aggregate | Every CI run; selected lanes must succeed and unselected lanes must skip | — | Seconds after dependencies |
| human-approval | Per-head `human approval` status | trusted auto-merge | Trusted PR lifecycle reset posts pending; eligible exact comment posts current-head success | — | Seconds |
| local-focused | Affected package, contract, or regression test and lint | local focused | Developer and Main feedback | — | Seconds to minutes |
| workspace-diagnosis | `scripts/verify.sh --workspace` | workspace diagnosis | Explicit cross-workspace diagnosis | — | Several minutes |
| release-version | Release-version check and regression | CI lightweight | Every PR and default-branch push | — | Seconds |
| product-docs | IoTKit product-docs profile check | CI lightweight | Every PR and default-branch push | — | Minutes |
| docs-regressions | Product-docs and documentation-script regressions | CI lightweight | Every PR and default-branch push | — | Seconds to minutes |
| trial-configuration | `python3 -m unittest scripts.tests.test_iotkit_trial` | CI lightweight | Every PR and default-branch push | — | Seconds |
| adapter-author-docs | Adapter author documentation API guard | CI lightweight | Every PR and default-branch push | — | Seconds |
| layer-rules | `scripts/check-layers` | CI lightweight | Every PR and default-branch push | — | Seconds |
| source-layout | `scripts/check-source-layout` | CI lightweight | Every PR and default-branch push | — | Seconds |
| battle-tested-routing | Battle-tested catalog and selector check | CI lightweight | Every PR and default-branch push | — | Seconds |
| product-docs-impact | Product-docs impact selector and regression | CI lightweight | Every PR and default-branch push | — | Seconds |
| ci-selector | `scripts/tests/select-ci-jobs.test.mjs` | CI lightweight | Every PR and default-branch push | — | Seconds |
| verification-policy | `scripts/tests/verification-policy.test.mjs` | CI lightweight | Every PR and default-branch push | — | Seconds |
| rust-edge-release-gates | Rust Edge release-gate contract regressions | CI lightweight | Every PR and default-branch push | — | Seconds |
| rust-concurrency | Local Rust concurrency defaults regression | CI lightweight | Every PR and default-branch push | — | Seconds |
| codex-cloud | `scripts/test-codex-cloud.sh` | CI lightweight | Every PR and default-branch push | 0 / 0 | Seconds |
| fmt | `cargo fmt --all --check` | CI Rust | Selector says Rust | — | Seconds |
| clippy | Selected-package or workspace Clippy | CI Rust | Selector says Rust | — | Minutes |
| rust-tests | Selected-package or workspace nextest suite | CI Rust | Selector says Rust | — | Minutes |
| rustdoc-tests | Selected-package or workspace Rust doc tests | CI Rust | Selector says Rust | — | Minutes |
| console-frontend | `scripts/test-edge-console-frontend.sh` | CI Console | Selector says Console | 0 / 0 | Minutes |
| console-e2e | `scripts/test-edge-console-e2e.sh` | CI Console | Selector says Console; host release also runs PostgreSQL | 1 / 0 | Minutes |
| rust-edge-custody | `scripts/test-rust-edge-custody.sh` | CI Edge | Selector says Edge; host release nests it through MQTT | 0 / 1 | Minutes |
| edge-output | `scripts/test-edge-output.sh` | CI Edge | Selector says Edge; host release also runs PostgreSQL | 1 / 0 | Minutes |
| edge-node-sigterm | `scripts/test-edge-node-sigterm.sh` | CI Edge | Selector says Edge | 0 / 0 | Minutes |
| trial-journey | `scripts/test-iotkit-trial.sh` | CI trial | Selector says trial | 0 / 0 | Minutes |
| certificate-hostname | `scripts/test-certificate-hostname.sh` | release | Host release gate | 1 / 0 | Minutes |
| recovery-acl | `scripts/test-edge-node-recovery-acl.sh` | field/manual | Recovery ACL upgrade rehearsal or its focused change | 0 / 0 | Seconds |
| edge-node-fence | `scripts/test-edge-node-fence.sh` | field/manual | Device retirement/fencing rehearsal or its focused change | 0 / 0 | Seconds |
| mqtt-security | `scripts/test-mqtt-security.sh` | release | Host release gate | 1 / 0 | Minutes |
| broker-cert | `scripts/test-broker-cert.sh` | release | Host release gate | 1 / 0 | Minutes |
| broker-cert-pebble | `scripts/test-broker-cert-pebble.sh` | release | Host release gate | 1 / 0 | Minutes |
| edge-mqtt | `scripts/test-edge-mqtt.sh` | release | Host release gate | 1 / 0 | Minutes |
| rust-edge-runtime | `scripts/test-rust-edge-runtime.sh` | release | Host release nests embedded and PostgreSQL runtime checks through MQTT | 0 / 2 | Minutes |
| edge-resilience | `scripts/test-edge-resilience.sh` | release | Host release gate | 1 / 0 | Minutes |
| edge-bootstrap | `scripts/test-edge-bootstrap.sh` | release | Host release gate | 1 / 0 | Minutes |
| edge-postgres | `scripts/test-edge-postgres.sh` | release | Host release gate and capacity's nested PostgreSQL mode | 1 / 1 | Minutes |
| edge-capacity | `scripts/test-edge-capacity.sh` | release | Host release gate | 1 / 0 | Several minutes |
| edge-parity | `scripts/test-edge-parity.sh surface` | field/manual | Edge replacement/parity rehearsal or its focused change | 0 / 0 | Seconds to minutes |
| host-release-gate | `scripts/test-edge-host-release-gate.sh NEW_REPORT_DIRECTORY` | release | Release candidate | root | Tens of minutes or more |
| hardware-power-cut | Physical hardware and power-cut evidence | field/manual | Field plan or incident follow-up | — | Human-scheduled |
