---
type: Contract
title: "IoTKit Edge Node recovery contract v1"
description: "Defines the sanitized encrypted Edge Node backup container, local fenced-candidate restore, operator boundaries, and disabled replacement surfaces."
language: en
translation_key: contracts.edge-node-recovery-v1
status: stable
revision: 1
---

# IoTKit Edge Node recovery contract v1

Status: **normative for the optional local Edge Node backup and slice-1
fenced-candidate restore boundary**.

This contract is the authority for the exact encrypted Node artifact and its
local restore boundary. It is paired with the schemas, fixtures, and
conformance tests linked below. A document, schema, fixture, or exported Rust
type never silently overrides another; disagreement is a contract defect.

## 1. Scope and shipped boundary

Slice 1 provides a local-root `iotkit-edge-nodectl backup` surface that can
create, inspect, and report the status of a custody-complete encrypted backup
of a sanitized Edge Node SQLite database. It also provides a restore operation
that accepts a schema-valid recovery handoff and installs a new local
candidate. The candidate is durably fenced before publication.

There is no backup configuration or enabled timer by default. The optional
systemd templates are inert until an operator installs them and explicitly
enables the timer. The service runs the exact CLI command in
`deploy/systemd/iotkit-edge-node-backup.service`
and the timer is
`deploy/systemd/iotkit-edge-node-backup.timer`.

Slice 1 does **not** ship production recovery-handoff creation, Broker fencing,
a remote permit, reconciliation, dedup-risk resolution, reactivation, or a
same-ID new ledger epoch. A valid handoff used by a conformance test or a
restore drill is not an operator-created authorization. An installed or
restored candidate remains unable to collect, publish, or bind the ingest
listener until a later, separately contracted permit and generation check
ships. No usable production replacement journey is claimed here.

## 2. Machine authority and conformance material

The paired machine artifacts are the executable wire authority:

| Material | Normative artifact |
| --- | --- |
| Container header schema | [schema](https://github.com/w-pinkietech/iotkit/blob/master/edge-node/core/recovery/contracts/node-backup-header-v1.schema.json) (`edge-node/core/recovery/contracts/node-backup-header-v1.schema.json`) |
| Sanitized manifest schema | [schema](https://github.com/w-pinkietech/iotkit/blob/master/edge-node/core/recovery/contracts/node-backup-manifest-v1.schema.json) (`edge-node/core/recovery/contracts/node-backup-manifest-v1.schema.json`) |
| Recovery handoff schema | [schema](https://github.com/w-pinkietech/iotkit/blob/master/edge-node/core/recovery/contracts/recovery-handoff-v1.schema.json) (`edge-node/core/recovery/contracts/recovery-handoff-v1.schema.json`) |
| Fenced restore receipt schema | [schema](https://github.com/w-pinkietech/iotkit/blob/master/edge-node/core/recovery/contracts/restore-receipt-v1.schema.json) (`edge-node/core/recovery/contracts/restore-receipt-v1.schema.json`) |
| Header golden | [fixture](https://github.com/w-pinkietech/iotkit/blob/master/edge-node/core/recovery/tests/fixtures/node-backup-header-v1.json) (`edge-node/core/recovery/tests/fixtures/node-backup-header-v1.json`) |
| Manifest golden | [fixture](https://github.com/w-pinkietech/iotkit/blob/master/edge-node/core/recovery/tests/fixtures/node-backup-manifest-v1.json) (`edge-node/core/recovery/tests/fixtures/node-backup-manifest-v1.json`) |
| Handoff golden | [fixture](https://github.com/w-pinkietech/iotkit/blob/master/edge-node/core/recovery/tests/fixtures/recovery-handoff-v1.json) (`edge-node/core/recovery/tests/fixtures/recovery-handoff-v1.json`) |
| Receipt golden | [fixture](https://github.com/w-pinkietech/iotkit/blob/master/edge-node/core/recovery/tests/fixtures/restore-receipt-v1.json) (`edge-node/core/recovery/tests/fixtures/restore-receipt-v1.json`) |
| Binary conformance vector | [fixture](https://github.com/w-pinkietech/iotkit/blob/master/edge-node/core/recovery/tests/fixtures/node-backup-v1.bin) (`edge-node/core/recovery/tests/fixtures/node-backup-v1.bin`) |
| Container conformance tests | [backup_contract.rs](https://github.com/w-pinkietech/iotkit/blob/master/edge-node/core/recovery/tests/backup_contract.rs) and [container_tests.rs](https://github.com/w-pinkietech/iotkit/blob/master/edge-node/core/recovery/tests/unit/container_tests.rs) |
| Restore conformance tests | [restore_tests.rs](https://github.com/w-pinkietech/iotkit/blob/master/edge-node/core/recovery/tests/unit/restore_tests.rs) and [recovery_startup.rs](https://github.com/w-pinkietech/iotkit/blob/master/edge-node/apps/node/tests/recovery_startup.rs) |

The checked-in binary is a public format vector only. Production encryption
uses operating-system randomness; its passphrase, key, paths, identifiers, and
digests are never copied into logs, errors, audit records, status output, or
debug representations.

## 3. Configuration, destination capability, and scheduling

`backup configure` writes a schema-1 owner-only configuration and its paired
systemd drop-in as one guarded publication. Configuration, passphrase, and
handoff files MUST be regular files owned by the invoking account, have one
link, and have no group or other permission bits (normally mode `0600`).
Existing configuration requires the explicit `--replace-existing` policy.

The destination is supported only after a capability probe succeeds. The
probe opens and holds the intended directory without following a symlink,
checks owner-only mode and writable capacity, checks no-replace publication,
file sync, parent-directory sync, and descriptor-relative read-back, and
rechecks the database and destination filesystem boundary. A filesystem name,
label, or mutable `/dev/sdX` spelling is not an endorsement. The persisted
mount identity is a stable block UUID (`uuid:<value>`) or a filesystem ID plus
decoded source (`fsid:<value>|<source>`); a missing stable identity fails
closed. The destination MUST be a different filesystem from the live database.

The staging capability has an existing, euid-owned, non-group/other-writable
`tmpfs` parent and an owner-only leaf. For the systemd path, `/run` is the
existing tmpfs parent (its usual `0755` mode is valid): `configure` opens it
without following the final component, validates its type and link count, and
records the exact staging path `/run/iotkit-edge-node-backup`. A world-writable
tmpfs root such as `/dev/shm` is not an accepted parent. `create` uses the held
parent descriptor to create an absent exact leaf with mode `0700`, or validates
an existing owner-only leaf's type, link count, and tmpfs filesystem. It MUST
NOT create or broaden an arbitrary `/run` tree, follow a substituted path, or
use the destination as staging. On restart the tmpfs staging contents disappear.

Install the optional templates only after reviewing the configured paths:

```sh
sudo install -D -m 0644 deploy/systemd/iotkit-edge-node-backup.service \
  /etc/systemd/system/iotkit-edge-node-backup.service
sudo install -D -m 0644 deploy/systemd/iotkit-edge-node-backup.timer \
  /etc/systemd/system/iotkit-edge-node-backup.timer
sudo install -d -m 0755 /etc/systemd/system/iotkit-edge-node-backup.service.d
sudo systemctl daemon-reload
```

The CLI generates only the following drop-in, using the captured mount point
from the capability check; do not hand-edit or substitute a mount name:

```ini
[Unit]
RequiresMountsFor=/absolute/captured/mount/point
```

After placing that exact file under
`/etc/systemd/system/iotkit-edge-node-backup.service.d/`, an operator may opt
in explicitly:

```sh
sudo systemctl enable --now iotkit-edge-node-backup.timer
sudo systemctl status iotkit-edge-node-backup.timer
```

`enable --now` is the activation decision; installation and daemon reload do
not enable the timer. `systemctl start iotkit-edge-node-backup.service` is a
manual one-shot check after configuration. A failed create is not an accepted
backup and does not authorize deletion of the live database.

## 4. Encrypted container framing

The artifact starts with the eight-byte ASCII magic `IOTKNDB1`. An Edge server
backup magic, an unknown artifact kind, an unknown version, or an unknown
algorithm is rejected. The magic is followed by a four-byte big-endian header
length and exactly that many header JSON bytes. The header length is nonzero
and at most 16 KiB. Header JSON is closed (`additionalProperties: false`), and
every byte of the magic, length, and exact JSON is authenticated.

The header fields and bounds are:

| Field | Exact value or bound |
| --- | --- |
| `artifact_kind` | `iotkit_edge_node_database` |
| `format_version` | integer `1` |
| `kdf` | `argon2id` |
| `salt_b64` | canonical unpadded Base64 of exactly 16 bytes (22 characters) |
| `kdf_time` | integer `1..=10` |
| `kdf_memory_kib` | integer `16,384..=262,144` |
| `kdf_parallelism` | integer `1..=16` |
| `cipher` | `xchacha20-poly1305` |
| `nonce_prefix_b64` | canonical unpadded Base64 of exactly 16 bytes (22 characters) |
| `chunk_size` | integer `4,096..=4,194,304` bytes |

The v1 writer defaults are fixed for newly created artifacts:

| Writer field | v1 default |
| --- | ---: |
| `kdf_time` | `3` |
| `kdf_memory_kib` | `65,536` KiB |
| `kdf_parallelism` | `4` |
| `chunk_size` | `262,144` bytes |

Readers MUST accept any value in the bounds above; the writer defaults are not
an additional reader restriction.

The key is 32 bytes derived with Argon2id (version 1.3) from the owner-supplied
passphrase and the authenticated salt/parameters. Salt and nonce prefix come
from operating-system randomness. Passphrase and derived key material are
zeroized; deterministic entropy exists only in test code and is not callable
by production code.

After the header, each record is:

```text
flags:u8 || plaintext_length:u32be || ciphertext_and_tag
```

Data records have `flags=0`, a nonzero plaintext length no larger than the
header `chunk_size`, and a 16-byte Poly1305 tag. There is exactly one terminal
record with `flags=1` and length zero. The terminal record is authenticated,
must occur after the manifest and database bytes, and must be followed by
immediate EOF. Unknown flags, truncation, a duplicate or early terminal,
zero-length data, oversized chunks, sequence overflow, malformed lengths, and
trailing bytes are invalid.

Record sequence numbers start at zero. The XChaCha20 nonce is the 16-byte
nonce prefix followed by the sequence as an unsigned 64-bit big-endian value.
Associated data is exactly `header_digest || sequence:u64be || flags:u8 ||
plaintext_length:u32be`, where `header_digest` is
`SHA-256(MAGIC || header_length:u32be || exact_header_json)`.

The authenticated plaintext stream is:

```text
manifest_length:u32be || manifest_json || sanitized_sqlite_bytes
```

The manifest length is nonzero and at most 1 MiB before allocation. The
manifest JSON is closed and validated before database bytes are accepted. The
database length and lowercase SHA-256 digest are computed while streaming and
must exactly equal the authenticated manifest before the terminal record can
be accepted. Authentication never creates plaintext; decryption writes only
to an anonymous owner-only staging file and never overwrites an existing path.

## 5. Sanitized manifest and database invariant

The manifest has `artifact_kind=iotkit-node-backup`, `format_version=1`,
`snapshot_mode=online`, `shutdown_seal_id=null`, and the current Edge Node
schema version (`23` in the checked-in vector). `backup_id`, `edge_node_id`,
and `ledger_epoch` are nonempty, at most 255 Unicode scalar values, contain no
colon or control character, and are not inferred from a pathname. Timestamps,
cursors, and allocation high-water are nonnegative signed 64-bit integers;
`accepted_cursor` MUST NOT exceed `allocation_high_water`. `database_length`
and every count are unsigned 64-bit integers. `database_sha256` is exactly 64
lowercase hexadecimal characters. The twelve closed count fields are
`devices`, `series`, `readings`, `publication_rows`, `ingest_dedup_rows`,
`staged_readings`, `quarantine_rows`, `device_principals`,
`device_credentials`, `activation_rows`, `ledger_events`, and `audit_events`.

The source is copied through the recovery snapshot operation, then the copy is
sanitized: `target_registry.credential_token` is cleared, journal mode is
DELETE, secure deletion is enabled, the copy is vacuumed, and `-wal`, `-shm`, and
`-journal` sidecars are absent. Canonical schema, integrity, identity, cursor,
and publication-boundary checks run before encryption and again on restore.
The artifact can contain authenticated readings and ingest-dedup claims up to
that snapshot boundary. The sanitizer removes the deployment credential token
from `target_registry`; account, session, and device credential hashes may
remain as protected database state. MQTT/TLS private material is outside this
database and is not placed in the artifact. Treat the encrypted artifact and
its passphrase as secrets.

## 6. Handoff, candidate binding, and idempotent recovery

`restore` accepts only a closed schema-1 `RecoveryHandoff`. Its required fields
are `recovery_id`, `edge_id`, `edge_node_id`, `old_ledger_epoch`,
`expected_backup_id`, `proposed_new_epoch`, and `credential_generation`, plus
`schema_version=1`. IDs use only ASCII letters, digits, `.`, `_`, and `-`, are
1..=255 bytes, and `old_ledger_epoch` MUST differ from
`proposed_new_epoch`. The generation is an integer
`0..=9,223,372,036,854,775,807`. The handoff MUST bind to the manifest's backup
ID, Node ID, and old epoch. Slice 1 records the nonnegative generation in the
candidate provenance and receipt; it does not compare that value with a live
authority or reject a generation mismatch. Authority comparison and activation
are deferred to the later permit/generation contract.

The public receipt is closed schema v1 with status
`durably_fenced_candidate` and fields `recovery_id`, `candidate_instance_id`,
`backup_id`, `edge_id`, `edge_node_id`, `old_ledger_epoch`,
`proposed_new_epoch`, and `credential_generation`. Candidate-row provenance
(source database length/digest and encrypted artifact length/digest) is bound
privately for replay and is never returned in the receipt, status, audit, or
errors.

The candidate target MUST be absent, owner-only, and separate from the live
database path after absolute normalization; equal names, aliases, symlinks,
hard links, and existing WAL/SHM sidecars fail closed. Restore publishes only
after offline validation and a typed install operation has entered the
`durably_fenced_candidate` state. The live database is never opened for write
and is never replaced by this operation.

An exact replay after rename (same authenticated artifact bytes, handoff, and
candidate binding) is a non-mutating reconciliation and returns the stored
receipt byte-for-byte. Different identity, handoff, artifact, or private
provenance returns `candidate_conflict`. A rename or later sync/read-back
uncertainty leaves the already-fenced candidate in place and returns
`candidate_publication_uncertain`; retrying the exact request is the only
supported reconciliation. No operation silently deletes a candidate to make a
retry pass.

## 7. Operator commands and restore-drill boundary

The local-root command shapes are:

```text
iotkit-edge-nodectl backup configure --config FILE --db DB \
  --destination DIR --staging-directory /run/iotkit-edge-node-backup \
  --passphrase-file FILE --freshness-seconds 86400 --retention-count 7 \
  --systemd-drop-in FILE [--replace-existing]
iotkit-edge-nodectl backup create --config /etc/iotkit/edge-node-backup.json
iotkit-edge-nodectl backup inspect --input FILE --passphrase-file FILE
iotkit-edge-nodectl backup status --config /etc/iotkit/edge-node-backup.json
```

Create, inspect, and status emit only bounded nonsecret summaries. Never put a
passphrase on an argument, in shell history, or in a log. Keep an encrypted
escrow copy of the passphrase under the deployment's approved owner-only
procedure; without it an artifact is intentionally unrecoverable. Verify a
successful artifact off-host and run an inspect/restore drill before relying
on its RPO.

The following is the conformance command shape, not a successful operator
procedure in slice 1:

```text
iotkit-edge-nodectl backup restore --input ARTIFACT \
  --candidate-db /secure/new/absent-candidate.db \
  --live-db CONFIGURED_LIVE_DB --passphrase-file PASSPHRASE_FILE \
  --recovery-handoff VALID_HANDOFF_FILE
```

The candidate path in a conformance run MUST be absent before the command and
MUST remain fenced afterward. A checked-in handoff fixture is conformance-only
and is valid only with the matching test-generated artifact; it MUST NOT be
paired with a selected real backup. Slice 1 has no later authority or matching
complete drill fixture, so a real-backup restore cannot succeed here and must
fail closed. Production handoff creation, Broker fencing, remote permit, and
reactivation are not shipped. A no-backup hardware replacement still restores
neither readings nor dedup claims; an encrypted-backup candidate contains
claims only through its authenticated snapshot boundary and remains fenced.

There is no legacy plaintext snapshot fallback. A former implementation's
artifact, a renamed Edge server backup, an unauthenticated database copy, or a
candidate with private MQTT/TLS material is not accepted by this contract.
