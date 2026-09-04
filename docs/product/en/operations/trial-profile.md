---
type: Runbook
title: "Try IoTKit on this PC"
description: "Starts, reviews, stops, and resets the loopback-only IoTKit trial profile."
language: en
translation_key: operations.trial-profile
status: draft
revision: 6
---

# Try IoTKit on this PC

This profile lets you watch IoTKit publish real Observations before you decide on a
field installation. It runs the IoTKit Edge Node and a standard Mosquitto Broker on
one Linux host and limits every network listener to IPv4 loopback. The generated
samples go through a regular Input Adapter and the pipelines to the topics of the
[MQTT Output Adapter contract v1](../contracts/mqtt-output-adapter-v1.md), where
`mosquitto_sub` subscribes to them. It is not a Console mock or a database seed.

The profile is for evaluation only. TLS, real sensors, and field updates are not
configured.

## Requirements

- A supported Linux host with Git and Python 3.14 or later.
- Docker Engine with the `docker compose` command.
- Local TCP port 18883 free.

## Start

Run from a clean repository clone.

```bash
./scripts/iotkit trial validate
./scripts/iotkit trial up
```

The first `up` builds the Edge Node image and takes a few minutes depending on the
host. If it is interrupted, running the same `up` again redoes the initialization.
Generated credentials and the database are stored outside the repository under
`${XDG_DATA_HOME:-$HOME/.local/share}/iotkit/trial` with owner-only permissions.

After the Edge Node starts, `up` imports these three pipeline definitions. The
edge-node-id is `trial`.

| pipeline-id | kind | input |
|---|---|---|
| `sample-illuminance` | `measurement` | trial illuminance (triangle wave, 120–200 lx, published on every input) |
| `sample-contact` | `state` | the trial contact state (square wave) after thresholding |
| `sample-cycles` | `accumulated-count` | rising edges of the same contact state |

## Shortest check

```bash
./scripts/iotkit trial watch
```

`watch` runs `mosquitto_sub` inside the Broker container with a read-only account and
prints every message as one line: topic, retain flag, payload. Stop it with Ctrl-C.
Check the following.

1. Heartbeats with `"value":"online"` and `"faults":[]` arrive on
   `iotkit/v1/edge-node/trial/status`.
2. On `.../observation/sample-illuminance/measurement`, `value` rises and falls in steps
   of 8 and `sequence` increases by one.
3. On `.../observation/sample-contact/state`, `value` alternates between `true` and `false`.
4. On `.../observation/sample-cycles/accumulated-count`, `value` starts at `0` and grows
   by one each time the state becomes `true`.
5. Running `watch` again first delivers the retained latest value (retain flag `1`),
   one per topic.

Stopping and restarting does not delete the database; series and sequence continue
across the restart.

```bash
./scripts/iotkit trial status
./scripts/iotkit trial down
./scripts/iotkit trial up
```

`trial down` gives the Edge Node a 15-second graceful-stop window. The Edge Node
publishes an `offline` status with the shutdown time before disconnecting; unsent
Observations stay in the outbox and arrive on the next `up`.

## Reset

Reset deletes only the recognized trial state directory and requires an explicit
data-loss confirmation.

```bash
./scripts/iotkit trial reset --confirm-trial-data-loss
```

For a field installation, stop the trial here and continue with
[Installation and recovery](installation-and-recovery.md). Trial state cannot be
promoted to a field environment.

## Changing the port

The two lines at the repository root are enough to start. Add settings only when the
default port collides with another process.

```toml
config_version = 1
profile = "trial"

[trial]
broker_port = 18884
sample_interval_ms = 1000
```

`broker_bind` accepts only IPv4 loopback addresses. Unknown keys, versions, profiles,
and non-loopback binds are rejected.
