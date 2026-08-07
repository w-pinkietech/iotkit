---
type: Runbook
title: "IoTKit Edge storage capacity regression smoke"
description: "Defines a repeatable capacity regression smoke for embedded SQLite and PostgreSQL profiles."
language: en
translation_key: operations.storage-capacity
status: stable
revision: 2
---

# IoTKit Edge storage capacity regression smoke

IoTKit does not promise unlimited scale merely because a profile is named `embedded` or `postgres`. Supported scale is determined by reproducible measurements that pin the product version, hardware, payload, rules, retention, and backup and query load.

```bash
scripts/test-edge-capacity.sh
```

The default profile supplies the same four Edge Nodes and eight sensors each to both profiles, retains
100,000 raw records per Edge Node (400,000 total), and reads one 100,000-record history. It creates
one numeric semantic rule per Edge Node after the retained prefix, accepts a 64-row matching tail per
node, creates an encrypted backup, restarts storage, then drains the 256 durable queue rows. During
that recovery it completes a real storage-status read only after projection has started and before the
queue reaches zero. This demonstrates that the scheduler leaves authoritative-storage work available;
it is not a latency SLA.

The JSON report records profile, raw count, records per second, batch-accept p99, history/backup/restart
and projection-recovery wall time, database bytes, semantic observations, queue lag before and after
recovery, pending output, failures, and foreground-storage completion. `projection_pending_before` and
`projection_pending_after` count `semantic_projection_queue` rows: durable rule-record work, not raw
records or receipt lag. The status implementation separately counts current `semantic_observations`,
`output_outbox`, and `semantic_projection_failures` rows. The script requires the full retained-history
profile and a zero queue after recovery, but timing values are evidence rather than portable pass/fail
thresholds. Capture CPU and RAM from the target host alongside the report; the Rust profile deliberately
does not invent a cross-platform CPU metric.

Never advertise a configuration as a verified scale without preserving its report.

This short smoke detects regressions; it does not prove production sizing or a support ceiling. Before a full deployment, reproduce the planned Edge Node and sensor counts, peak records/s, average payload, semantic-rule count, retention days, CSV and graph usage, external Broker outage, backup, and restart. Record at least:

- `accepted-through` p99 and unacknowledged backlog;
- semantic-projection queue lag/recovery and output-outbox latency;
- database/WAL size, free space, and daily growth;
- CPU, RAM, history query time, 100,000-row CSV time, and backup time;
- restart time and cursor/hash consistency after forced termination.

If a deployment exceeds the measured SQLite envelope, stop and migrate the same IoTKit Edge to the PostgreSQL profile. Never make SQLite and PostgreSQL simultaneous authorities or dual-store raw data in TimescaleDB.
