---
type: Runbook
title: "Edge Node hardware recovery quick guide"
description: "A field decision guide and printable checklist for replacing a failed Edge Node host."
language: en
translation_key: operations.edge-node-hardware-recovery
status: stable
revision: 2
---

# Edge Node hardware recovery quick guide

Use this page when replacing a failed Edge Node host. It is a field checklist,
not a replacement for the authoritative commands in
[Installation and recovery §8.1](installation-and-recovery.md#81-return-an-edge-node-encrypted-backup-candidate-to-production)
or the [Edge Node recovery contract](../contracts/edge-node-recovery-v1.md).
Run the complete detailed procedure in order; do not copy isolated commands
from it.

Scheduled backup is optional. The selected recovery path depends on whether a
usable encrypted backup and its passphrase exist.

## Stop conditions

Keep the replacement candidate fenced and stop when any of these is true:

- the old host is still powered, connected, or able to use its Broker
  credential;
- the selected encrypted artifact cannot be authenticated with the escrowed
  passphrase;
- a required Broker, IoTKit Edge, or candidate acknowledgement is unavailable;
- the final report does not show `state=completed`,
  `completion_acknowledged=true`, and `cursor_converged=true`;
- local ownership has not been re-established; or
- the operator does not have authority to accept the no-backup loss boundary.

An outage may delay the procedure; it never authorizes bypassing a fence or
starting the normal runtime. Never put a credential, token, key, passphrase,
hash, or customer identifier in the incident record.

## Choose the path

| Situation | Path | Data boundary |
| --- | --- | --- |
| An authenticated encrypted backup and its passphrase are available | Follow the backup-available checklist and the complete §8.1 procedure | Readings and deduplication claims are restored only through the authenticated snapshot boundary. A later local tail may still be unprovable. |
| The old host is healthy and this is a planned replacement | First create, authenticate, and retain an off-host encrypted backup using §7.1; then use the backup-available path | The newly authenticated snapshot becomes the recovery boundary. |
| No authenticated encrypted backup or passphrase is available | Follow the no-backup checklist | Readings and deduplication claims are not restored. This is a clean replacement with an explicitly accepted loss boundary, not a restore. |

Do not treat a legacy snapshot, plaintext database copy, SQL edit, or invented
handoff as a backup.

## Field checklist: before replacement

- [ ] Open an incident record and assign one person to control the recovery.
- [ ] Record the Edge Node ID and non-secret evidence needed to identify the
      last known state.
- [ ] Stop and physically isolate the old host.
- [ ] Fence the old Broker credential and retain the non-secret fence receipt.
- [ ] Preserve the old host and database as incident evidence; do not erase or
      reuse them.
- [ ] Identify the deployed runtime user/group, live database path, deployment
      owner, and actual supervisor unit.
- [ ] Choose one path from the table above and record why.

## Field checklist: backup available

- [ ] Authenticate and inspect the selected artifact before restore.
- [ ] Run the complete
      [§8.1 production-return procedure](installation-and-recovery.md#81-return-an-edge-node-encrypted-backup-candidate-to-production)
      in order, restoring only to a new candidate path.
- [ ] Keep the candidate fenced while IoTKit Edge authorizes the exact
      candidate and new ledger epoch.
- [ ] Retain the final report proving `state=completed`,
      `completion_acknowledged=true`, and `cursor_converged=true`.
- [ ] Reset the local owner passphrase interactively. If authenticated HTTP
      ingest is used, reapply its desired listener, TLS generation, and device
      authority through the normal typed operations.
- [ ] Rebind the existing backup configuration to the recovered database with
      `--replace-existing`.
- [ ] Create and authenticate a fresh backup, confirm healthy backup status,
      retain it off host, and authenticate the retained copy with the same
      backup ID.
- [ ] Start the deployed normal runtime only after every required gate passes.
- [ ] Record `remaining_gap_review_required` and the explicit possible-loss
      boundary in the incident review.
- [ ] Retire the old host without re-enabling its credential or database.

## Field checklist: no backup

- [ ] Record that no authenticated encrypted backup and usable passphrase are
      available.
- [ ] Record that readings and deduplication claims cannot be restored.
- [ ] Obtain the site's required approval for that loss boundary before
      commissioning a clean replacement.
- [ ] Keep the old host and storage as evidence; a later verified source may
      change the incident decision.
- [ ] Do not run the encrypted-backup recovery flow, alter cursors with SQL, or
      claim continuity from the failed Node.
- [ ] Plan and verify clean commissioning and the later new ledger epoch as a
      separate operation. Require downstream idempotency to expose any possible
      duplicate; do not describe the result as restored continuity.

## Clean replacement identity result

After the no-backup loss boundary is approved, follow the normal
[installation procedure](installation-and-recovery.md#1-install) with a fresh database and a new
`edge_node_id`. Generate a new MQTT binding and credential; never reuse the fenced credential,
database, recovery handoff, or candidate from the failed identity. A retained `recovery_hold` and
its evidence do not need to be deleted before the separate new Node is commissioned.

When the new Node reports a sensor—even the same physical sensor on the same port with the same
measurement type—IoTKit Edge creates a new `device_ref` and `signal_ref`. The new signal starts
without a display profile, semantic rules, calibration, or output binding and is configured through
the normal post-registration flow. The old signal, settings, and history remain attached to the old
Edge Node identity. With no new observations they become operationally stale; clean replacement
does not automatically retire or delete them, merge history, or claim continuity.

Record the new Edge Node ID and new signal ref as clean-commissioning evidence. Record the old ID
and loss boundary separately so an operator cannot mistake the two histories for one continuous
sensor.

## Incident closure evidence

For backup recovery, do not close the incident until the record contains:

- the old-host isolation and Broker fence receipt;
- the selected artifact identity and successful authentication result;
- the recovery ID and final report;
- proof that local ownership was re-established;
- the fresh post-recovery backup ID, healthy status, and authenticated
  off-host copy; and
- the cursor convergence result and explicit remaining-gap decision.

For a no-backup replacement, mark backup-specific evidence as unavailable
rather than fabricating it, and record the approved loss boundary and clean
commissioning evidence. This checklist is suitable for printing, but secrets
must remain in their approved owner-only storage.
