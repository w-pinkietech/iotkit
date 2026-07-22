---
type: Contract Overview
title: "IoTKit ingest contract v1 overview"
description: "Entry point for the contract that idempotently delivers observations from a device or adapter to an Edge Node."
language: en
translation_key: contracts.ingest-v1
status: stable
revision: 1
---

# IoTKit ingest contract v1 overview

This document is an orientation guide to the boundary. In the same Git revision, `docs/ingest-contract.md` is the detailed authority for fields, limits, and error semantics. Treat it, exported Rust wire types, shared fixtures, and conformance tests as one contract artifact set.

The ingest contract is the boundary where a sender delivers one `Envelope` to the Edge Node collector and receives an `Ack` containing positional item results. A retry preserves the same immutable Envelope, the same `envelope_id`, and the same payload. A sender must not reuse an ID for changed content.

The sender supplies facts such as observed subject, measurement key, channel, value, and device time. The receiver derives the authenticated source or principal from the binding; a sender-controlled field in the payload never grants authority. The collector validates the measurement registry, value type, bounds, ledger, and duplicate state before storage.

The current bindings are:

* Official in-process Input Adapters use the shared ingest client.
* Contract-native or external devices use the default-off TLS `POST /api/v1/ingest` endpoint with a device credential.

HTTP requests have fixed limits for body size, item count, strings, and concurrent admission. `/validate` applies the same validation without writing. A successful HTTP response still requires inspection of every item result. A transient storage failure is not converted into a deterministic `rejected` result; it remains a failure for which the sender may retry the same Envelope.

A disagreement among those artifacts is a contract defect and does not make one artifact the automatic winner.
