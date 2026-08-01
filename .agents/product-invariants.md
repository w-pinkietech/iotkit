# Product invariants

- Never expose tokens, credentials, keys, their hashes, customer identifiers, or
  sensitive configuration in debug output, logs, errors, audit records, fixtures,
  issues, or pull requests.
- Never silently lose data. Follow the current ingest and custody contracts.
  `rejected` is only for deterministic terminal violations. A storage failure
  does not produce `rejected` or a durable success acknowledgement.
- Mutations go through the owning typed operation dispatcher: `edge-node/core/ops`
  on Edge Node and `edge/src/application/` on IoTKit Edge. Do not add API/UI/CLI
  paths that write SQL directly.
- Do not treat MQTT PUBACK as IoTKit Edge durable raw acceptance, or downstream
  business success as IoTKit output custody.

Return to [`AGENTS.md`](../AGENTS.md).
