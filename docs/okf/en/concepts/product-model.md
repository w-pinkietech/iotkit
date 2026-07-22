---
type: Concept
title: "IoTKit product model"
description: "Defines the scope of IoTKit across sensor collection, preservation, semantics, and external output."
language: en
translation_key: concepts.product-model
status: stable
revision: 1
---

# IoTKit product model

IoTKit is a reusable IoT foundation placed between sensors and factory or business applications. It connects varied sensors, preserves observations through network and power failures, lets an operator assign generic meaning, and converts observations into versioned messages for external applications.

IoTKit does not own factories, products, processes, work orders, OEE, production results, or business alarms. Those belong to applications such as YokaKit. BravePI and YokaKit are the first verified integrations, but neither defines the IoTKit core model.

IoTKit provides three central values:

1. An Edge Node continues collecting and durably buffering while upstream systems are unavailable.
2. Input Adapter, ingest, custody, and Output Adapter boundaries are public and versioned.
3. Plant operators can use the IoTKit Edge Console to inspect, query, and export current values and history, and to manage display, semantic, and external-output settings. Setting changes do not rewrite existing raw data or semantic history.

One IoTKit Edge can manage multiple Edge Nodes. Management across multiple `edge_id` values belongs to an optional fleet or business layer above IoTKit.

See [Terminology](terminology.md) and the [System overview](../architecture/system-overview.md).
