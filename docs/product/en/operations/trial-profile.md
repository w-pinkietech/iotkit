---
type: Runbook
title: "Try IoTKit on this PC"
description: "Starts, reviews, stops, and resets the loopback-only IoTKit trial profile."
language: en
translation_key: operations.trial-profile
status: draft
revision: 3
---

# Try IoTKit on this PC

Use this profile to see the real IoTKit collection loop before making field
deployment decisions. It runs IoTKit Edge Node, a standard Mosquitto Broker, and
IoTKit Edge on one Linux host. All network listeners are restricted to IPv4
loopback. The generated sample follows the normal Input Adapter and custody
contracts; it is not a Console mock or a database seed.

This profile is for evaluation only. It does not configure TLS, backup, physical
sensors, PostgreSQL, high availability, or a field-ready update process.

## Requirements

- A supported Linux host with Git and Python 3.11 or later.
- Docker Engine with the `docker compose` command.
- Free local TCP ports 8080 and 18883.

## Start

From a clean repository clone:

```bash
./scripts/iotkit trial validate
./scripts/iotkit trial up
```

The first `up` builds the trial image, so it can take several minutes depending
on the host. If the command is interrupted, run the same `up` again to restart
initialization.

At the prompt, choose a trial administrator password of 12 to 128 characters.
The launcher does not put the password in `iotkit.toml`, command arguments, or
output. Generated credentials and databases are stored with owner-only
permissions under `${XDG_DATA_HOME:-$HOME/.local/share}/iotkit/trial`, outside
the repository.

Open `http://127.0.0.1:8080` and sign in with login ID `admin` and the password
you chose. The yellow **Trial environment** banner must remain visible.

## Shortest review

1. On **Overview**, confirm that one Edge Node was detected.
2. Open **Equipment**, select the Edge Node, and activate it.
3. Open the trial illuminance sensor and the trial contact-state sensor, and complete any prompted display settings.
4. On **Sensors**, confirm both series:
   - illuminance (continuous triangle wave) changes slowly
   - contact state (square wave) toggles High / Low (`1` / `0`)
5. Open **Received history** and confirm that rows for both series increase.

The trial sample adapter emits both series from the same `trial-sample` instance through
the normal Input Adapter and custody path. Values are not seeded into the database or
Console. No extra waveform configuration is required; the two-line `iotkit.toml` enables
both by default.

Activation remains explicit because it is part of the real custody contract.
Stopping and starting the trial does not delete its databases:

```bash
./scripts/iotkit trial status
./scripts/iotkit trial down
./scripts/iotkit trial up
```

## Reset

Reset deletes only the recognized trial state directory and requires an explicit
data-loss confirmation:

```bash
./scripts/iotkit trial reset --confirm-trial-data-loss
```

To deploy at a site, stop here and use
[Installation and recovery](installation-and-recovery.md). Trial state is not
promoted into a field deployment.

## Optional ports

The two-line root configuration is sufficient. If the defaults conflict with
another local process, add only the required settings:

```toml
config_version = 1
profile = "trial"

[trial]
console_port = 18080
broker_port = 18884
sample_interval_ms = 1000
```

`console_bind` and `broker_bind` may be set only to an IPv4 loopback address.
Unknown keys, versions, profiles, and non-loopback binds are rejected.
