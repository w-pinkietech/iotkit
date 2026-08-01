---
type: Runbook
title: "IoTKit Edge storage capacity regression smoke"
description: "Defines a repeatable capacity regression smoke for embedded SQLite and PostgreSQL profiles."
language: en
translation_key: operations.storage-capacity
status: stable
revision: 1
---

# IoTKit Edge storage capacity regression smoke

IoTKit does not promise unlimited scale merely because a profile is named `embedded` or `postgres`. Supported scale is determined by reproducible measurements that pin the product version, hardware, payload, rules, retention, and backup and query load.

```bash
scripts/test-edge-capacity.sh
```

The smoke supplies the same four Edge Nodes, eight sensors each, and 8,000 total raw records to both profiles. It runs normal batch acceptance, reads up to 8,000 history rows, and creates an encrypted backup. The JSON report records profile, records per second, batch-accept p99, query and backup duration, database bytes, pending output, and projection failures. Never advertise a configuration as a verified scale without preserving its report.

This short smoke detects regressions; it does not prove production sizing or a support ceiling. Before a full deployment, reproduce the planned Edge Node and sensor counts, peak records/s, average payload, semantic-rule count, retention days, CSV and graph usage, external Broker outage, backup, and restart. Record at least:

- `accepted-through` p99 and unacknowledged backlog;
- semantic-projection and output-outbox latency;
- database/WAL size, free space, and daily growth;
- CPU, RAM, history query time, 100,000-row CSV time, and backup time;
- restart time and cursor/hash consistency after forced termination.

If a deployment exceeds the measured SQLite envelope, stop and migrate the same IoTKit Edge to the PostgreSQL profile. Never make SQLite and PostgreSQL simultaneous authorities or dual-store raw data in TimescaleDB.
