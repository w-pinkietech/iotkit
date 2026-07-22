---
type: Contract Overview
title: "IoTKit Input Adapter contract v1 overview"
description: "Entry point for the contract that composes sensor-specific implementations into a generic Edge Node host."
language: en
translation_key: contracts.input-adapter-v1
status: stable
revision: 1
---

# IoTKit Input Adapter contract v1 overview

This document is an orientation guide to the boundary. In the same Git revision, `docs/input-adapter-contract.md`, the exported host API, and testkit form the detailed contract artifact set for types, lifecycle, configuration, and conformance.

An Input Adapter is the accountable integration unit that connects one vendor, transport, or device model to IoTKit. It delivers observations to the common ingest contract without leaking vendor vocabulary into the IoTKit core.

Responsibilities are separated as follows:

* A transport backend handles only raw I/O such as serial, I2C, or GPIO.
* A device driver owns protocol, registers, detection, initialization, and datasheet-derived physical conversion.
* Adapter runtime and composition own driver execution, lifecycle, and mapping to measurement keys and channels.
* The ingest client owns Envelopes, ID allocation, delivery, Ack handling, and retry.
* The Edge Node host owns configuration authority, principal creation, series resolution, storage, restart policy, and health aggregation.

Adapter type, configured instance, diagnostic source, authenticated principal, observed subject, and IoTKit system identity are distinct identities. An Envelope whose source does not match the source bound by the host is not accepted.

A host platform such as Raspberry Pi 4B or 5 is used for capability checks, but is not part of adapter, source, or device identity. BravePI is one implementation; other adapters and drivers can use the same host contract.

An adapter does not own storage, custody cursors, semantic rules, Output Adapters, or external Broker credentials.
