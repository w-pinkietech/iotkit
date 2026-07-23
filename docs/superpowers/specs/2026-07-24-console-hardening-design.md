# Rust Console Hardening Design

## Goal

Complete the Rust Console boundary so device and signal presentation profiles survive reloads, semantic and export previews execute the same evaluator and Output Adapter policies as production, and binding publication views report durable outbox and PUBACK state. The real-browser gate must start the unchanged production runtime with an authenticated ephemeral MQTT Broker.

## Authority and compatibility

The current product model, Output Adapter v1 contract, Console OpenAPI, Go Console behavior, and the Rust Task 6 semantic/output model define the required behavior. The Go schema is an oracle for presentation fields and response semantics, but obsolete Go semantic-v3 tables are not copied into Rust.

Known Console and API mutations must use typed application operations. Web handlers do not issue SQL. Operational writes remain restricted to active administrator or system-administrator principals; viewers may read saved previews but may not submit draft semantic settings.

## Durable inventory and presentation profiles

Migration `0006_console_profiles.sql` is added to both SQLite and PostgreSQL. It creates:

- `inventory_devices(device_ref, edge_node_id, system_id, created_at)` with stable unique descriptor identity;
- `inventory_signals(signal_ref, edge_node_id, series_key, system_id, created_at)` with stable unique descriptor identity;
- `device_profiles(edge_node_id, system_id, display_name, location, revision, updated_at)`;
- `signal_profiles(edge_node_id, series_key, display_name, display_sensor_type, display_sensor_type_label, display_value_kind, display_unit_mode, display_unit, decimal_places, revision, updated_at)`.

The migration backfills inventory rows for existing descriptor replicas. Descriptor application creates stable inventory rows for newly discovered devices and signals without replacing existing refs. Semantic rule creation reuses the inventory signal ref, so adding the first rule never changes a Console URL.

`InventoryProfiles` owns validation and authorization. Updates trim display text, enforce the Go field constraints, require the current revision when a profile already exists, increment revisions, and insert a bounded audit summary in the same transaction. Inventory reads join profiles and descriptors so saved names, units, decimal formatting, location, and completion state feed SSR, JSON history, and output labels after process restart.

## Semantic preview

`Semantics::preview` resolves the stable signal, reads a bounded recent raw window, decodes finite scalar measurements, and calls `semantics::build_preview`. A draft request uses the submitted calibration and rules; a saved request loads current calibration and active rules. Each rule is evaluated independently so one invalid rule is returned with its own error without hiding successful siblings.

The response follows the OpenAPI `MappingPreview` or `MultipleRuleMappingPreview` shape, including input/plot counts, points, test result, window bounds, and threshold metadata. Preview operations are read-only.

## Export activation and binding publication preview

`OutputProfiles::preview_activation(adapter_id)` uses the registered adapter descriptor and compatible observation modes to classify every active semantic rule as `automatic`, `needs_configuration`, or `ineligible`. It uses saved signal display names and performs no writes.

`OutputProfiles::publication(binding_id)` loads the durable route and returns:

1. the newest durable outbox topic/payload when present (`provenance=actual`);
2. otherwise a transformation of the latest persisted semantic observation (`latest_observation`);
3. otherwise a contract-valid sample observation transformed through the registered adapter (`sample`).

The same read includes durable delivery facts: pending and published counts, oldest pending time, last published time, and a derived delivery state. Pending younger than five minutes is `delivering`; pending at least five minutes is `possible_delivery_stall`; no pending with a published row is `delivered`; otherwise it is `waiting_for_observation`. MQTT PUBACK remains the only transition that increments published state.

## Browser and backend verification

SQLite integration tests prove profile revision, audit atomicity, reload behavior, evaluator-backed semantic preview, adapter-policy activation preview, and publication fallback/delivery transitions. PostgreSQL runs the same profile and preview contract when `IOTKIT_TEST_POSTGRES_DSN` is provided.

The browser fixture seeds a descriptor, raw records, semantic rule, and output profile. The browser changes device and signal presentation profiles, semantic calibration/rule settings, and output state, then reloads pages and verifies persisted values and durable delivery facts.

The E2E script starts an ephemeral authenticated Mosquitto, creates owner-only runtime and Broker password files, and passes `--edge-id`, `--broker-url`, `--username`, `--password-file`, `--allow-insecure`, and development HTTP flags to the actual production `serve` command. It never relaxes production CLI validation. PostgreSQL browser execution remains environment-gated.

## Failure and security behavior

Invalid fields return bounded field-specific 400 errors. Missing resources return 404. Missing or stale revisions return precondition/conflict errors without writes. Storage or audit failure rolls back the entire mutation. Preview transformation failures disclose no secrets or stored setup values. Broker credentials are file inputs, owner-only, and absent from browser responses and diagnostics.
