---
type: Runbook Overview
title: "IoTKit installation and recovery overview"
description: "Entry point for safely installing, checking, and recovering IoTKit Edge Nodes, Brokers, and IoTKit Edge."
language: en
translation_key: operations.installation-and-recovery
status: stable
revision: 1
---

# IoTKit installation and recovery overview

This document summarizes operational principles. In the same Git revision, use `docs/edge-operations.md` for authoritative commands, file modes, and stop conditions on failure.

## Installation order

1. Initialize the Edge Node and inspect its generated identity and non-secret MQTT binding.
2. On the IoTKit Edge host, generate deployment material from the Broker hostname, bind address, existing TLS certificate, private key, trust bundle, and Edge Node binding.
3. Distribute per-Edge-Node credentials, ACLs, and CA trust securely. Never place secrets in argv, environment variables, logs, or Git.
4. Select exactly one authoritative IoTKit Edge storage profile, `embedded` or `postgres`. Never fall back to another backend, including during startup; fail startup or stop if the selected profile cannot be opened.
5. Create the first system administrator through a local-only bootstrap operation and sign in to the Console.
6. Confirm that Broker enrollment grants transport access only, discover the Edge Node in the Console, and complete the exact incarnation's activation request, Edge Node application, and matching-result commit. Then verify that a later commissioning smoke record reaches `accepted-through`.

The Broker and IoTKit Edge may share a host or use separate hosts. DNS, LAN, firewall, VPN, and certificate issuance are deployment responsibilities. Manage Console HTTPS termination and MQTT Broker TLS as separate boundaries.

## Daily checks

Inspect structured status for Edge Nodes, Broker, IoTKit Edge, the authoritative database, certificate expiry, pending custody, pending external output, and backups. An MQTT PUBACK alone is not end-to-end success.

## Recovery principles

An encrypted backup includes identity, cursors, configuration, and outbox state consistently. Restore while stopped into a new path or empty database, validate ownership, profile, schema, identity, and cursors, and only then return traffic. Moving from SQLite to PostgreSQL is also an offline migration and never a dual-write operation.

Recover lost account authority with a local host operation. IoTKit exposes neither an unauthenticated network setup route nor an HTTP fallback. Certificates renew automatically through the issuer and renewal client selected at installation; monitor expiry and renewal failure.
