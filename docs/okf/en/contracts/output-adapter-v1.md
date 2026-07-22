---
type: Contract Overview
title: "IoTKit Output Adapter contract v1 overview"
description: "Entry point for the contract that transforms a generic Observation into an application-specific MQTT publication."
language: en
translation_key: contracts.output-adapter-v1
status: stable
revision: 1
---

# IoTKit Output Adapter contract v1 overview

This document is an orientation guide to the boundary. In the same Git revision, `docs/output-adapter-contract.md`, exported Go types, and shared fixtures form the detailed contract artifact set for types, configuration schemas, errors, and bindings.

An Output Adapter is an in-process transformer. It takes a generic IoTKit Edge Observation plus versioned route configuration and deterministically returns one MQTT publication for one external application.

Generic Observation kinds are:

| kind | value | meaning |
|---|---|---|
| `numeric` | finite JSON number | calibrated or transformed numeric value |
| `boolean` | JSON boolean | generic on/off state |
| `cumulative_value` | non-negative JSON integer | accumulated value since an origin |
| `alarm` | JSON boolean | alarm raised or cleared |

`production` and `gantt_chart` are application purposes, not IoTKit core kinds. An adapter does not recreate the input `observation_id`, `series_id`, sequence, timestamp, or value.

The adapter validates configuration schema version and capabilities, then returns exactly one topic, payload, QoS, and retain setting, or a typed error. The same input and configuration produce the same result. IoTKit Edge owns credentials, Broker connectivity, retry, the durable outbox, and semantic evaluation.

`iotkit.mqtt-json.v1` emits every generic kind without changing its meaning. `yokakit.mqtt.v1` transforms supported kinds into the YokaKit purpose-bound contract and does not guess an application purpose for unsupported `numeric` observations. Another external service is added as another adapter behind the same boundary.
