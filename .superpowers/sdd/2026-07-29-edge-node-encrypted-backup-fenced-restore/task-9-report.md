# Task 9 — Optional Edge Node scheduling and recovery authority

Issue: #113

## Scope

This task adds the optional systemd service/timer and their template contract,
a real temporary-drop-in test, the bilingual Edge Node recovery contract, and
the operator/ingest wording for the shipped slice-1 boundary. It does not add a
production handoff producer, Broker fence, remote permit, activation, or a
usable replacement journey.

## RED (before unit templates)

The test was added before either unit existed:

```text
node --test scripts/tests/edge-node-backup-systemd.test.mjs
✖ backup service has the owner-only runtime staging contract
  Error: ENOENT ... deploy/systemd/iotkit-edge-node-backup.service
✖ backup timer is opt-in and uses the daily jitter contract
  Error: ENOENT ... deploy/systemd/iotkit-edge-node-backup.timer
﹣ nodectl configure pins the exact captured mount point in a temporary drop-in
  # requires a runnable Linux nodectl binary; Rust backup_cli covers the same contract on other hosts
2 failed, 1 skipped
```

The third case is not a self-authored fixture: on a Linux host with a runnable
`iotkit-edge-nodectl`, it invokes `backup configure` against owner-only temporary
directories, reads the persisted captured mount point, and asserts the generated
drop-in is exactly `RequiresMountsFor=<captured mount point>`. The Windows RED
run skipped only that Linux executable path.

## GREEN

Windows host:

```text
node --test scripts/tests/edge-node-backup-systemd.test.mjs
2 passed, 0 failed, 1 skipped (Linux product invocation)
```

WSL Ubuntu host:

```text
node --test scripts/tests/edge-node-backup-systemd.test.mjs
3 passed, 0 failed, 0 skipped
```

The WSL run exercised the real configure path and exact generated drop-in. The
service template contains `Type=oneshot`, `UMask=0077`,
`RuntimeDirectory=iotkit-edge-node-backup`, `RuntimeDirectoryMode=0700`,
`Environment=TMPDIR=/run/iotkit-edge-node-backup`, and the exact Task 9
`ExecStart`. The timer is opt-in with `OnCalendar=daily`,
`RandomizedDelaySec=2h`, `Persistent=true`, and `WantedBy=timers.target`.

## Documentation authority

`docs/okf/en/contracts/edge-node-recovery-v1.md` and its Japanese pair define:

- `IOTKNDB1`, exact big-endian framing, closed/bounded header and manifest,
  Argon2id/XChaCha20-Poly1305 parameters, nonce/AAD construction, record and
  terminal invariants;
- sanitized manifest fields/counts and the `target_registry` deployment-token
  sanitizer (credential hashes may remain protected encrypted state);
- closed handoff/receipt schemas, candidate-row provenance binding, absent-path
  restore, fenced startup, exact post-rename replay, and conflict/uncertainty
  behavior;
- machine schema, golden fixture, binary vector, and conformance-test paths;
- default-off scheduling, owner-only files, capability-tested mount identity,
  `/run` tmpfs parent versus exact `RuntimeDirectory=` leaf, manual commands,
  escrow, and restore-drill limits.

The bilingual runbook distinguishes Edge Node sections 7.1/8.1 from existing
IoTKit Edge sections 7.2/8.2. The ingest contract now says that encrypted
custody-complete sanitized backup and local fenced-candidate restore are
implemented, while Broker fencing, remote permit, reconciliation, dedup-risk
resolution, reactivation, and same-ID new epoch remain deferred/default-off.
It also states that backup candidates carry claims only through the authenticated
snapshot boundary and that no-backup replacement restores neither.

## Verification

- `node --test scripts/tests/edge-node-backup-systemd.test.mjs` (Windows:
  2 passed, 1 host skip; WSL: 3 passed).
- `node scripts/check-okf-docs.mjs` — passed (10 bilingual concepts).
- `scripts/check-layers` — passed on Windows.
- `scripts/check-source-layout` — passed on Windows.
- `node scripts/battle-tested-review.mjs check` — passed (5 entries).
- `node scripts/battle-tested-review.mjs select --base origin/master` selected
  BT-002 (physical power-loss durability) and BT-004 (computer replacement
  loss boundary), in addition to the broader branch's BT-001/003 routes.
- `git diff --check` — passed.
- `systemd-analyze verify` was attempted in WSL; it cannot validate this
  repository copy because Windows checkout permissions mark units executable/
  world-writable and `/usr/local/bin/iotkit-edge-nodectl` is not installed.
  This is deployment evidence still required on a target host, not a template
  test failure.

## Residual / deferred

Physical power-cut, SD-card/filesystem-controller durability and deployment
mount capability remain release/field gates. Slice 1 intentionally has no
production handoff creator, Broker fencing, permit, reconciliation, dedup
activation, reactivation, same-ID new epoch, or usable replacement procedure.
The candidate stays fenced and cannot collect or publish. No push or merge was
performed.
