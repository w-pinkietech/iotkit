---
type: Concept
title: "IoTKit terminology"
description: "Defines the main terms used for IoTKit components, identities, and delivery responsibility."
language: en
translation_key: concepts.terminology
status: stable
revision: 1
---

# IoTKit terminology

| Term | Definition |
|---|---|
| Device | An endpoint that measures or detects a physical condition, including sensors and transmitters. |
| Input Adapter | A component that translates vendor- or protocol-specific input into the generic ingest contract. |
| IoTKit Edge Node | A runtime near devices that collects, normalizes, durably buffers, and retries observations. |
| Internal MQTT Broker | A standard Broker that transports messages between Edge Nodes and IoTKit Edge. It is not the authority for custody. |
| IoTKit Edge | An aggregation service that durably stores data from multiple Edge Nodes and provides the Console, semantics, history, and external output. |
| Output Adapter | A component that deterministically transforms a generic observation into one external application's topic and payload. |
| custody | Responsibility for preserving data. It transfers from an Edge Node only after IoTKit Edge durably commits the data. |
| `edge_id` | The identifier for one IoTKit Edge scope. It is not a factory identifier. |
| `edge_node_id` | The identifier for one Edge Node. |
| series | The identity under which observations of the same subject and measurement remain continuous over time. |
| observation | One timestamped value with a type and identity. |
| quarantine | A state in which data is stored but excluded from external delivery and rule evaluation until released. |

“Gateway” is not a formal product component name. An IoTKit Edge Node is more than a relay because it owns a durable buffer, while IoTKit Edge performs aggregation, semantic configuration, and external output.
