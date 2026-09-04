---
type: Runbook
title: "IoTKit Edge storage capacity regression smoke"
description: "Defines a repeatable capacity regression smoke for embedded SQLite and PostgreSQL profiles."
language: en
translation_key: operations.storage-capacity
status: stable
revision: 6
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

The settings real-signal preview resolves the selected signal once and uses its profile's raw
`(edge_node_id, series_key, received_at DESC, ledger_epoch DESC, pub_seq DESC)` index. Its raw read
is therefore bounded by the requested tail (`1..=2000`), rather than a JSON extraction and sort of
all retained raw history. SQLite stores the full key in that index; PostgreSQL uses a fixed-length
`md5(series_key)` discriminator and then rechecks the full key. This keeps long retained keys within
the PostgreSQL raw-preview B-tree tuple limit without allowing a digest collision to change preview
results.

Schema v12 also adds a latest-only Edge Node status row plus current-epoch raw-receipt and active
rule/route diagnostic indexes. Those indexes do not backfill or copy history rows, but their build
reads retained raw, observation, and outbox history. Include the associated startup time and temporary
database/WAL footprint in the capacity smoke at the deployment's retained-history scale.

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

## Device-side outbox (device-local redesign)

After the redesign in [#232](https://github.com/w-pinkietech/iotkit/issues/232), the only thing that can keep growing in the device's SQLite is the outbox of unsent publications. Time series are not stored on the device.

- One row is about 200 bytes (topic, payload, a little metadata).
- `accumulated-count` and `state` publish only on change. `measurement` publishes only when the calibrated value changes, so its worst case equals the input rate.
- Example: one `measurement` pipeline fed every second with a value that always changes grows by about 86,400 rows, roughly 17 MB, per day while the Broker is down. At 250 ms that is about 70 MB per day.
- While the Broker is reachable, each PUBACK deletes a row, so the steady-state outbox stays at a few rows.

The first version has no outbox size limit or thinning. On sites where Broker outages last long, compare the free disk space with the input rate of `measurement` pipelines and lower the rate if needed.
