# Console Causal Diagnostics Design

Date: 2026-08-09

Issue: [#93](https://github.com/w-pinkietech/iotkit/issues/93)

Status: implemented and verified

## 1. Purpose

The Console currently reports an inventory signal as receiving after any value
has ever been stored. When data later stops, an operator cannot tell whether
the sensor path, Input Adapter, Edge Node, internal MQTT path, IoTKit Edge
ingest, semantic projection, or external output is the first layer requiring
attention.

This change gives a non-IT operator one causal, evidence-backed status on the
existing `/status` page. It reuses durable custody, projection, and output
facts and transports the bounded Edge Node health facts that are currently
available only in its local `health.json`.

## 2. Scope and non-goals

This design covers:

- a secret-free Edge Node status heartbeat over the existing MQTT boundary;
- latest-only SQLite and PostgreSQL persistence for the last accepted status;
- process-local IoTKit Edge MQTT connection and subscription state;
- deterministic stage classification, severity, recovery, and next checks;
- an accessible server-rendered causal section on `/status`;
- explicit separation of registration from operational state on `/equipment`;
- diagnostics API and OpenAPI parity;
- paired product contracts and operating guidance;
- deterministic failure and recovery evidence using existing harnesses.

This design does not:

- infer that an event-driven sensor is stopped from data age alone;
- expose raw errors, endpoints, credentials, topics, payloads, filesystem
  paths, or configuration through the heartbeat or Console;
- add a second diagnostics page, a client polling framework, or manual incident
  dismissal;
- duplicate binding, pending-row, or PUBACK detail owned by `/output`;
- keep the Console available after a critical IoTKit Edge task exits. A fatal
  ingest, projection, storage, or HTTP task stops the supervised service and is
  diagnosed through the host process manager and logs;
- prove physical wiring, process-manager, router, DNS, Wi-Fi, or field network
  behavior through automated fixtures.

## 3. Evidence boundary

The classifier may use only these facts:

- latest raw receipt and accepted cursor maintained by IoTKit Edge;
- a newly received Edge Node status sequence;
- current IoTKit Edge MQTT connection and confirmed subscription state;
- semantic queue, failure, and later-success evidence;
- output transform state and durable outbox/PUBACK evidence.

Descriptor receipt and activation mean registered, not online. A retained
heartbeat replay is historical evidence and does not establish current
liveness. An `unknown` stage never aggregates to healthy. A downstream stage
whose input is absent is `blocked_by` or `no_new_input`, not independently
failed.

## 4. Edge Node status contract

### 4.1 Topic and delivery

The Edge Node publishes to:

`iotkit/v1/edge-nodes/{edge_node_id}/status`

The message uses MQTT QoS 1 and retained delivery. The normal heartbeat period
is 30 seconds. The status publication is operational evidence only: MQTT
PUBACK does not advance reading custody, and status publication failure never
changes the reading cursor.

The existing per-node ACL gains write access to that exact topic and IoTKit
Edge gains wildcard read access. Topic identity and payload `edge_node_id`
must match. Packet and collection bounds are part of the contract.

No last-will message is added. An ungraceful stop is detected by the absence of
a newer heartbeat; graceful shutdown behavior remains owned by the process
lifecycle rather than a second offline protocol.

### 4.2 Version 1 payload

The version 1 payload contains only:

- `schema_version` (exactly `1`);
- `edge_node_id` and active `ledger_epoch`;
- a random process `boot_id` and monotonically increasing `status_seq`;
- `collector_state`: `running` or `stopped`;
- up to 64 adapters with bounded opaque `adapter_id` and one of `running`,
  `restarting`, `exhausted`, or `stopped`;
- the Edge Node view of `accepted_through` and bounded pending publication
  count;
- `storage_pressure`: a boolean derived from the existing local watermark.

It intentionally contains no producer timestamp. IoTKit Edge receipt time is
the freshness authority. Adapter error strings and `last_error` are not
transported; the local health file remains the detailed host diagnostic.

Unknown fields, unsafe identities, duplicate adapter IDs, invalid enums,
negative cursors/counts, excessive arrays/text/encoding, and topic identity
mismatch fail closed. The Edge Node and IoTKit Edge implementations share
checked-in golden fixtures and independent encode/decode conformance tests.

### 4.3 Replay and restart semantics

For a non-retained live publication, IoTKit Edge accepts:

- a greater `status_seq` for the stored `boot_id`; or
- the first live sequence from a different `boot_id`.

A duplicate or lower sequence does not refresh liveness. A retained
publication may populate historical details when no row exists, but never sets
or refreshes `last_live_received_at` and never overwrites a newer live row.
The active Edge Node and ledger epoch must match the activation authority.

## 5. Storage and runtime state

Schema version 12 adds one latest-only status row per Edge Node in both SQLite
and PostgreSQL. It stores normalized contract fields, the latest message
receipt, and nullable latest live receipt. The migration has transactional
upgrade/rollback, backup/restore, and SQLite-to-PostgreSQL parity evidence.
There is no heartbeat history table and no retained-history scan.

IoTKit Edge also owns a small process-local MQTT health value with these closed
states:

- `unknown`: no connection result has been observed;
- `connecting`: connection or subscription is in progress;
- `ready`: ConnAck and every required subscription were confirmed;
- `disconnected`: a previously attempted connection is unavailable.

The runtime records only a coarse stable state and last ready time. It never
stores broker error text. Subscription readiness follows SubAck, not merely a
queued subscribe call. The web application receives a read-only snapshot of
this state.

## 6. Causal status model

The `/status` section presents these stages in operator reading order:

1. Sensor input
2. Input Adapter
3. Edge Node collector
4. Internal Broker path
5. IoTKit Edge ingest and raw custody
6. Semantic projection
7. External output

Each stage has a closed state (`ok`, `warning`, `critical`, `unknown`, or
`not_applicable`), a stable code, last success when known, bounded affected
scope/count, cautious likely cause, and one verb-led next check. Only the
earliest actionable non-healthy stage receives the prominent action. Actions
link to the existing equipment, logs, system, or output surfaces.

Classification proceeds from transport evidence toward downstream work:

- MQTT `ready` is healthy. `unknown` remains unknown; `connecting` is warning;
  `disconnected` is critical and is described as an internal Broker/network/
  TLS/authentication path problem without guessing which one.
- A live status younger than 90 seconds is fresh. Three missed heartbeats
  (90 seconds) are warning and ten missed heartbeats (300 seconds) are
  critical. A retained-only snapshot is unknown. A newer live sequence clears
  heartbeat staleness.
- A fresh explicit stopped collector is critical. A fresh `restarting`
  adapter is warning; `stopped` or `exhausted` is critical. A newer running
  snapshot clears the incident.
- The latest signal raw receipt older than five minutes is an advisory warning
  only when the upstream Broker, Edge Node, and adapter evidence is healthy.
  It says no new data was received; it does not claim the sensor is stopped.
  New raw data clears it.
- A fresh Node status with pending publications while the Edge cursor does not
  progress marks ingest/raw custody warning. Closed contract rejection codes
  may identify an inactive node, recovery fence, sequence gap, conflict, or
  invalid contract. Cursor progress clears it. Application acceptance remains
  distinct from MQTT PUBACK.
- An eligible projection head older than five minutes is warning. A terminal
  projection failure is critical until a later successful observation for the
  same active rule proves recovery; the historical failure remains available
  in diagnostics/log evidence.
- External output reuses the existing five-minute pending-outbox/PUBACK warning
  and transform error facts. It clears when the blocked transform succeeds or
  the oldest pending row receives PUBACK. `/output` remains authoritative.

If an upstream stage is not healthy, stages that require its new input report
`blocked_by` or `no_new_input`; old success timestamps alone do not create
downstream incidents.

## 7. Console and API

The new causal section is server rendered after commissioning state and before
general metrics. It is a labelled ordered list; every state is written as text
and never communicated by color alone. The layout stacks at narrow widths and
keeps the single primary action at least 44 pixels high.

The existing diagnostics endpoint gains typed stage results. Existing general
issues remain available. The implementation also aligns its closed OpenAPI
schema with the fields already emitted by storage and broker-certificate
diagnostics; it must not paper over current schema drift by allowing arbitrary
properties.

Equipment pages relabel inventory state as `登録状態` and state explicitly that
registration does not mean online. Operational state comes only from the
causal evidence above.

## 8. Verification

Implementation starts with focused failing tests for:

- wire bounds, topic identity, duplicate/lower/replayed sequences, and secret
  rejection;
- SQLite/PostgreSQL schema 11 to 12 migration, rollback, backup/restore, and
  copy parity;
- confirmed MQTT subscription state and reconnect recovery;
- mutually distinguishable Broker, Node, adapter, raw custody, projection, and
  output states, including exact clearing transitions;
- event-driven raw age remaining advisory and unknown never becoming healthy;
- accessible Console ordering, wording, single primary action, and registered
  versus online text;
- OpenAPI/runtime parity.

Existing resilience, output, and Console browser harnesses are extended rather
than replaced. BT-005 remains a hypothesis until an unfamiliar operator
completes the physical journey; its automated guards and coverage gap are
updated. BT-001 is selected for custody/Broker/output fault paths. Product
contracts and recovery guidance are updated in English and Japanese together.
