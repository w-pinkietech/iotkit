# Selective CI Design

Status: approved implementation design for issue #79. This file records process
rationale and does not override the current product corpus under `docs/okf/`.

## Goal

Run inexpensive repository consistency checks for every pull request and protected
branch push, while starting Rust and IoTKit Edge test jobs only when their results
can be affected by the changed files.

## Jobs

The existing `CI` workflow remains one workflow with stable job names:

1. `changes` checks out enough history, lists changed paths, and emits `rust` and
   `edge` boolean outputs.
2. `lightweight` always runs OKF documentation, layer, source-layout, and
   battle-tested review checks.
3. `rust` runs the existing Rust dependency installation, format, Clippy, build,
   and workspace tests only when `changes.outputs.rust == 'true'`.
4. `edge` runs the existing Go and Console checks only when
   `changes.outputs.edge == 'true'`.

A skipped heavy job is an intentional successful outcome for an unrelated diff.
The lightweight job and change-classification job still appear on every run.

## Change classification

Classification is implemented by a repository script with a small, testable
interface: it receives one changed path per line and prints GitHub-output
compatible `rust=true|false` and `edge=true|false` values.

- Documentation and repository-guidance paths select neither heavy job.
- Rust workspace source, manifests, lockfile, toolchain, adapters, Edge Node, and
  Rust verification infrastructure select `rust`.
- `iotkit-edge/`, its deployment assets, and Console/Go verification
  infrastructure select `edge`.
- Shared fixtures or scripts that exercise both sides select both.
- Workflow files, the classifier itself, and unknown paths select both. This is
  the safe fallback and prevents a newly added component from silently escaping
  CI.

The exact path table lives with the classifier tests, not as duplicated workflow
expressions.

## Event ranges

For a pull request, changed paths are computed from the pull request base SHA to
the checked-out head. For a push, they are computed from the event's `before` SHA
to `after`. A missing or unusable base is treated as unknown and selects both
heavy jobs.

The checkout used by `changes` fetches full history so these ranges are available.

## Verification

Focused tests cover at least:

- documentation-only;
- Rust-only;
- IoTKit Edge/Console-only;
- shared change selecting both;
- workflow and classifier changes selecting both;
- unknown paths selecting both;
- empty or malformed input selecting both.

The workflow YAML is then checked against the script outputs, and the existing
lightweight checks are run locally. Product test suites do not need to be rerun
merely because their steps were moved unchanged between CI jobs.

## Non-goals

- Changing product behavior or public contracts.
- Rewriting or reducing existing Rust, Go, or Console test coverage.
- Changing branch-protection settings.
- Adding a third-party changed-files action.
