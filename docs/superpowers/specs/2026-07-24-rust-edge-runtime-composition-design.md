# Rust Edge Runtime Composition Design

## Goal

Make `iotkit-edge serve` own the complete production runtime lifecycle instead
of validating flags and returning immediately. One composition root connects
storage, MQTT ingest, semantic projection, optional MQTT output, and Axum HTTP
to one cancellation domain. A signal requests graceful shutdown; any critical
task failure, panic, or unexpected clean exit fails the process.

## Typed configuration

`ServeArgs` converts once into a secret-redacting `RuntimeConfig`.

- Storage is selected through the existing deployment-profile validation and
  connected before tasks are created.
- MQTT URLs accept only `ssl://host:port` in production or `tcp://host:port`
  when the matching `--allow-insecure` flag is explicit.
- Trust mode is a closed enum. `system_roots` forbids a CA file;
  `bundle_only` requires and reads a CA file; plaintext forbids TLS settings.
- Passwords remain owner-only file inputs. Config `Debug` output exposes only
  whether credentials are present.
- Output configuration is optional as a whole. Supplying any partial output
  option without `--output-broker-url` is rejected.
- HTTP listen and public origin are parsed into typed socket/origin values.

No task receives `ServeArgs` or reads deployment files independently.

## Composition boundary

`RuntimeFactory` is the only replaceable construction boundary. It creates the
production `WebApplication` from the connected `Storage`. The default CLI
factory currently returns `WebAdapterUnavailable`; composition returns that
error before spawning any task. Tests inject a real test `WebApplication`.

When the storage-backed Task 7 adapter is available, only the default
factory's `web_application(Storage)` method changes.

The factory does not abstract storage, MQTT, semantic evaluation, or
supervision. Those are production implementations in both tests and runtime.

## Runtime tasks

After all fallible construction succeeds, the composition root creates one
`CancellationToken` and registers these critical tasks:

1. MQTT ingest `IngestRuntime`.
2. Semantic projection loop, which repeatedly calls `project_pending` and
   waits on cancellation or a bounded idle interval.
3. MQTT output `OutputRuntime` when output broker configuration is present.
4. Axum HTTP listener using the injected `WebApplication`.
5. Unix signal listener for SIGINT and SIGTERM.

The signal listener is a shutdown trigger, not a critical worker. It cancels
the shared token. No task is detached.

## Supervision and shutdown

The supervisor owns every join handle. A requested signal cancels the shared
token and starts a bounded drain. Successful completion requires every task to
join within the shutdown deadline.

For a critical task, all of these are process failures:

- returning an error;
- panicking;
- returning `Ok(())` before cancellation.

On the first critical failure the supervisor cancels siblings, drains them,
and returns the failing task name. Failure remains the process result even if
sibling shutdown succeeds. A drain timeout aborts remaining owned handles and
returns `ShutdownTimedOut`.

## HTTP behavior

The HTTP task binds before reporting itself ready and uses
`axum::serve(...).with_graceful_shutdown(...)`. Bind errors are critical
startup failures. The runtime never claims HTTP availability without a
`WebApplication`.

## Verification

Focused tests cover:

- typed MQTT URL, trust, secret-file, and redacted-debug validation;
- missing production Web adapter fails before any task starts;
- signal cancellation joins all tasks;
- error, panic, and unexpected clean critical exits fail the process;
- semantic projection stops on cancellation and treats storage failure as
  critical.

The real gate starts Mosquitto and the composed runtime with SQLite using the
same flags as the deployment command. It publishes descriptor, activation, and
record traffic, then proves raw custody, semantic projection, MQTT output
PUBACK marking, HTTP liveness, and graceful SIGTERM. PostgreSQL runs the same
gate when its test profile is selected.
