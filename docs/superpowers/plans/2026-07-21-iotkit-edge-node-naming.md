# IoTKit Edge / Edge Node Rename Implementation Plan

**Goal:** Replace the IoTKit Site/Edge hierarchy with IoTKit Edge/IoTKit Edge Node across contracts,
storage, runtime artifacts, deployment, documentation, and Console without compatibility aliases.

**Architecture:** The Go aggregate becomes `iotkit-edge` and owns the `edge_id` state domain. The Rust
collector becomes `iotkit-edge-node` and retains `edge_node_id`; MQTT custody topics and canonical
record identity remain unchanged. Existing development databases are intentionally reset rather than migrated.

## Global constraints

- `Edge` never means the collecting node; use `IoTKit Edge Node`, `Edge Node`, or UI label `収集ノード`.
- `edge_id` identifies the aggregate state domain; `edge_node_id` identifies a collector.
- Keep `iotkit/v1/edge-nodes/{edge_node_id}/...` and `(edge_node_id, ledger_epoch, pub_seq)` unchanged.
- Keep MQTT Broker independently deployable.
- Do not add runtime aliases, legacy routes, legacy JSON fields, or old database migration support.
- Do not rewrite historical specs and plans other than this design and plan.
- Preserve custody, acknowledgement, authentication, and typed-dispatch invariants from `AGENTS.md`.

## Task 1: Canonical terminology and contracts

- Update current architecture, terminology, responsibility, custody, authentication, and adapter documents.
- Define `edge_id` as the IoTKit Edge state domain and preserve `edge_node_id` as node identity.
- Define Standalone, Edge-connected, Edge Node activation, and Fleet layer.
- Keep MQTT Broker a separate transport dependency.

## Task 2: Rust Edge Node runtime

- Move `iotkit-edge/` to `iotkit-edge-node/` and `iotkit-edgectl/` to `iotkit-edge-nodectl/`.
- Rename package, binary, library crate, configuration, TLS identity, MQTT client ID, and operator text.
- Change activation JSON from `site_id` to `edge_id` and activation state to Edge Node terminology.
- Preserve Edge Node topics, ledger epoch, cursor, publication sequence, and accepted-through behavior.

## Task 3: Go IoTKit Edge runtime and persistence

- Move `iotkit-site/` to `iotkit-edge/`, including module, command, packages, cookies, and static assets.
- Rename `site_id` to `edge_id` and `site_meta` to `edge_meta`.
- Rename every child-node type, route, reference, table, and diagnostic component to explicit Edge Node forms.
- Update backup metadata, audit operations, output source identity, and fresh database initialization.

## Task 4: Deployment and credentials

- Rename Compose, bootstrap/add scripts, tests, environment files, data paths, and CI calls.
- Use disjoint role-qualified central principals such as `iotkit-edge-archive-<edge_id>` and
  `iotkit-edge-output-<edge_id>`; use `iotkit-edge-node-<edge_node_id>` for node clients.
- Keep generic Broker and output configuration names where they describe roles rather than products.
- Verify clean bootstrap, ACL negatives, certificate tooling, and Compose syntax.

## Task 5: OpenAPI and Console

- Rename child routes to `/api/v1/edge-nodes` and `/equipment/edge-nodes`.
- Expose explicit `edge_node_id` fields where the node source is part of the operator contract.
- Regenerate TypeScript from `openapi/edge-console-v1.yaml`.
- Use `IoTKit Console`, `IoTKit Edge`, and `収集ノード` consistently.
- Remove factory/worksite assumptions and clarify that pre-registration values are not sent or included
  in IoTKit Edge history.

## Task 6: Verification

- Scan active non-historical files for old products, routes, environment keys, and ambiguous child `Edge` names.
- Run Rust formatting, layer checks, workspace tests, and clippy.
- Run Go tests, generated frontend checks, shell syntax checks, bootstrap/MQTT/output/resilience tests, and Console E2E.
- Run `scripts/verify.sh` once as the final full verification.
- Review the complete diff against the design and custody/security invariants before reporting completion.
