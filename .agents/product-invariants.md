# Product invariants

- Never expose tokens, credentials, keys, their hashes, customer identifiers, or
  sensitive configuration in debug output, logs, errors, audit records, fixtures,
  issues, or pull requests.
- Never silently lose data. Follow the current ingest and custody contracts.
  `rejected` is only for deterministic terminal violations. A storage failure
  does not produce `rejected` or a durable success acknowledgement.
- Mutations go through the owning typed operation dispatcher (`edge-node/core/ops`).
  Do not add API/UI/CLI paths that write SQL directly.
- MQTT PUBACK is the boundary of IoTKit's delivery responsibility: the outbox
  row may be deleted after it, and nothing more is promised. Do not treat PUBACK
  as the consumer having stored or processed the Observation.

Return to [`AGENTS.md`](../AGENTS.md).
