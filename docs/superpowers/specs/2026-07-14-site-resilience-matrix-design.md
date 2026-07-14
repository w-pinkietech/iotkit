# IoTKit Site resilience matrix design

Date: 2026-07-14
Status: Approved for implementation

## Authority and purpose

The current implementation gate and its product invariants remain authoritative in
`docs/redesign/decisions/D3-process-and-wave-decisions.md`. This document defines only
how to close the gate's remaining failure-testing work. It does not create a new wire,
storage, or custody contract.

The goal is to prove that IoTKit Edge keeps custody until IoTKit Site has durably
accepted a contiguous prefix, including across storage failure, conflicting replay,
downstream outage, and component restart. Most verification runs on the host machine.
Raspberry Pi testing is reserved for behavior that depends on the UART and real
BravePI hardware.

## Scope

This work adds:

- deterministic, fast Go tests for Site transaction and acknowledgement invariants;
- a host-side Docker resilience script for the Edge/Broker/Site process boundary;
- fast Site tests in normal CI;
- a small, hardware-specific Raspberry Pi confirmation after host tests pass; and
- final evidence in D3 after the host and hardware checks have run.

This work does not add:

- production failpoints, test-only production configuration, or a fault-injection API;
- exhaustive restart permutations;
- privileged network manipulation, disk filling, or filesystem corruption;
- literal multi-hour waits;
- new retention, backup, restore, semantic mapping, or application-export behavior; or
- broader BravePI configuration, pairing, or downlink control.

## Chosen approach

Testing is split at the boundary where each failure can be made deterministic.

### Fast Site tests

The real Site SQLite store is used for transaction semantics. SQLite triggers provide
a deterministic write failure inside the acceptance transaction. The MQTT processor
is tested with a controlled publisher so an error can be proven not to emit an
`accepted-through` message.

This deliberately does not claim to fail the filesystem at the exact `COMMIT` syscall.
It proves the custody invariant that matters at the public boundary: when the Site
acceptance transaction cannot commit, none of its writes survive and no application
acknowledgement is emitted. Exact OS-level commit fault injection remains outside this
deterministic matrix.

The fast suite covers:

1. a cursor write failure rolls back both raw-record and cursor writes;
2. any store failure prevents `accepted-through` publication;
3. an altered replay at the same `(edge_node_id, ledger_epoch, pub_seq)` returns a
   content conflict, retains the original row, and does not advance the cursor;
4. the conflict path through the processor emits no acknowledgement; and
5. an exact replay remains idempotent and creates no duplicate raw row.

These tests run under ordinary `go test ./...` and belong in normal CI. Existing tests
that already prove part of an invariant are strengthened or composed instead of being
duplicated under new names.

### Docker resilience matrix

A new `scripts/test-site-resilience.sh` owns the slower black-box matrix. The existing
`scripts/test-site-mqtt.sh` remains the short happy-path vertical slice. Only small,
stable environment setup may be shared if that materially reduces duplication.

The resilience script uses a unique Compose project name and a private temporary
directory. Credentials remain in mode-restricted temporary files and are never printed.
The script builds or reuses the normal Edge and Site artifacts; no special product
binary is created.

The test seeds 300 valid Edge publications while delivery is unavailable. This crosses
the egress limit of 256 records and therefore proves convergence over more than one
batch without waiting for hours. Direct database seeding is confined to the test
harness: the scenario under test starts at the durable Edge outbox and exercises the
normal MQTT egress and Site acceptance path.

The ordered matrix is:

1. initialize clean Edge, Broker, and Site identities and storage;
2. make the Broker unavailable and durably seed 300 contiguous Edge publications;
3. restart Edge while downstream remains unavailable;
4. verify the Edge outbox persists and its Site cursor has not advanced;
5. restore Broker and Site, then wait for Site raw storage and Edge cursor to converge
   through publication 300;
6. stop Site while leaving Broker available, add more contiguous publications, and
   verify transport availability alone does not advance the Edge cursor;
7. restart Edge with that unacknowledged backlog, restore Site, and verify complete,
   duplicate-free convergence;
8. restart Edge, Broker, and Site individually, publishing a new record after each
   restart to prove the next delivery succeeds; and
9. verify both SQLite databases with `PRAGMA quick_check`, Site row count and contiguous
   `pub_seq`, and equality of the final Site and Edge accepted-through cursors.

Stopping the Broker represents loss of the Edge-facing MQTT path. Stopping Site while
the Broker remains available separately proves that MQTT PUBACK is not application
custody. The matrix intentionally covers each component restart and the important mixed
case of an Edge restart during downstream outage; it does not enumerate every ordering
of simultaneous restarts.

### Raspberry Pi confirmation

The Pi phase starts only after the host suite passes. It uses the existing experimental
host and the SSH safeguards in `AGENTS.md`. Service changes, process restarts, and any
physical BravePI Mainboard action require confirmation at the time of the test.

The hardware-only checks are:

1. confirm BravePI Mainboard frames are arriving over UART;
2. make downstream unavailable while real sensor observations continue to enter the
   Edge SQLite database;
3. restart Edge and verify UART collection resumes automatically;
4. restore downstream and verify every new observation reaches Site and the Edge cursor
   converges; and
5. when the user can perform the physical action, restart BravePI Mainboard and verify
   UART output resumes without repairing IoTKit state.

General SQLite, conflict, and MQTT restart cases are not repeated on the Pi.

## Failure behavior and diagnostics

Every wait is bounded. A timeout fails the scenario and prints only the diagnostics
needed to locate the stalled boundary: component logs, process/container state, record
counts, and cursor values. Secret contents, password files, private keys, and full
credential-bearing configuration are excluded.

The Docker script installs an exit trap before it creates processes or containers. The
trap stops the Edge process, removes the unique Compose project and volumes, and deletes
the temporary directory on both success and failure. Cleanup errors do not conceal the
original test failure.

A failed Site transaction or a content conflict must leave the batch unacknowledged.
The Edge retry loop may repeat the batch, but it must not move the cursor until a valid,
correlated `accepted-through` is received. Exact retries may update no business state
other than harmless operational timestamps.

## CI and verification economy

Normal CI adds the fast, deterministic Site Go suite. The Docker resilience matrix is
not a per-commit CI job; it is run once before the pull request, together with the
project's final full verification. This preserves quick feedback while still requiring
black-box evidence before integration.

During implementation, focused tests run after the corresponding change. The complete
Rust workspace verification, complete Go suite, existing MQTT vertical slice, and new
Docker resilience matrix run once at the final pre-PR gate, consistent with the project
verification policy.

## Expected change boundary

Expected files are limited to:

- `iotkit-site/internal/store/store_test.go`;
- `iotkit-site/internal/mqttsite/processor_test.go`;
- `.github/workflows/ci.yml`;
- `scripts/test-site-resilience.sh`;
- narrowly shared script support only if necessary; and
- D3 for final observed evidence after execution.

Product Store, Processor, Edge, wire-contract, and schema code should remain unchanged.
If a test exposes a real defect there, implementation stops and the defect is diagnosed
before this design is expanded.

## Completion criteria

The work is complete when:

- fast Site tests prove rollback, no-ack-on-error, conflict preservation, and exact
  replay idempotency;
- those fast Site tests run in normal CI;
- the Docker matrix proves multi-batch outage accumulation, persistent retry across
  Edge restart, no cursor advance from transport acknowledgement, individual component
  restart recovery, duplicate-free convergence, and healthy databases;
- the required UART and real-sensor recovery checks pass on Raspberry Pi;
- D3 records the exact host and hardware evidence without secrets; and
- the final pre-PR verification passes once in full.
