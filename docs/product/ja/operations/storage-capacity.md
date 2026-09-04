---
type: Runbook
title: "IoTKit Edge storage容量回帰smoke"
description: "Embedded SQLiteとPostgreSQL profileの再現可能な容量回帰smoke手順を定義します。"
language: ja
translation_key: operations.storage-capacity
status: stable
revision: 7
---

# IoTKit Edge storage capacity regression smoke

IoTKitは、`embedded`または`postgres`という名前だけで無制限の規模を保証しない。
対応規模は、対象version、hardware、payload、rule、保持期間、backup/query負荷を固定した
再現可能な計測結果で判断する。

```bash
scripts/test-edge-capacity.sh /secure/report/directory
```

既定profileは両profileへ同じ4 Edge Nodes、各8 sensorを用い、Edge Nodeごとに100,000 raw record
（合計400,000件）を保持して一つの100,000件historyを読む。保持prefix後に各Edge Nodeへ一つのnumeric
semantic ruleを作り、nodeごとに64件のmatching tailを受理し、暗号化backup、storage restart、256件の
durable queue rowのdrainを順に行う。Recovery中はprojection開始後かつqueueがzeroになる前に実際の
storage status読出しを完了させる。これはschedulerがauthoritative storage workを進められることの証拠であり、
latency SLAではない。

設定のreal-signal previewは選択したsignalを一度だけ解決し、profileのraw
`(edge_node_id, series_key, received_at DESC, ledger_epoch DESC, pub_seq DESC)` indexを使います。したがってraw読出しは要求tail（`1..=2000`）にboundedされ、保持済みraw history全体へのJSON抽出とsortにはなりません。SQLiteはこのindexにfull keyを保持します。PostgreSQLは固定長の`md5(series_key)` discriminatorを使った後に完全なkeyを再照合するため、長い保持keyでもPostgreSQL raw-preview B-tree tuple limit内に収まり、digest collisionでpreview結果は変わりません。

Schema v12は、latest-only Edge Node status rowに加え、current epoch raw receiptとactive rule/route診断indexを追加します。これらのindexはhistory rowをbackfillもcopyもしませんが、構築時に保持済みraw、Observation、outbox historyを読みます。deploymentの保持history量でcapacity smokeを行う際は、そのstartup時間と一時的なdatabase/WAL footprintを含めます。

JSON reportにはprofile、raw件数、records/s、batch受理p99、history/backup/restart/projection recoveryの
wall time、DB bytes、semantic observation、recovery前後のqueue lag、pending output、failure、foreground
storage completionを残す。`projection_pending_before`と`projection_pending_after`は
`semantic_projection_queue` row、すなわちraw record件数やreceipt lagではないdurable rule-record workを数える。
Status実装は現行の`semantic_observations`、`output_outbox`、`semantic_projection_failures` rowも別々に数える。
Scriptはfull retained-history profileとrecovery後queue zeroを必須にするが、時間値はportableなpass/fail
thresholdではなくevidenceである。CPU/RAMはtarget hostでreportと同時に採取する。Rust profileはcross-platformな
CPU metricを捏造しない。

reportを保存していない構成を「検証済み規模」として案内してはならない。

この短いsmokeは回帰検知用であり、実導入のsizingや対応上限を証明しない。本格導入前には予定する
Edge Node数、sensor数、ピークrecords/s、平均payload、意味付けrule数、保持日数、CSV/graph利用、
外部Broker停止、backup、restartを再現し、少なくとも次を記録する。

- accepted-through p99と未ack backlog
- 意味付けprojection queueのlag/recoveryとoutput outboxの遅延
- DB/WAL容量、空き容量、増加量/日
- CPU、RAM、履歴query時間、100,000件CSV時間、backup時間
- 強制終了後の再起動時間とcursor/hash整合性

SQLiteで上限を超える場合は、同一IoTKit Edgeを停止移行してPostgreSQL profileへ切り替える。
SQLiteとPostgreSQLを同時に正本にしたり、TimescaleDBへrawを二重保存したりしない。

## 端末側のoutbox（端末完結の再設計）

[#232](https://github.com/w-pinkietech/iotkit/issues/232) の再設計後、端末のSQLiteで増え続けうるのは未送信のpublicationを持つoutboxだけである。時系列は端末に保存しない。

- 1行は約200バイト（topic、payload、少数のメタデータ）。
- `accumulated-count`と`state`は変化があったときだけ公開する。`measurement`は入力ごとに公開するので、その公開頻度は入力の頻度に等しい。
- 例：1秒周期の入力を持つ`measurement` pipeline 1本は、Broker停止中に1日で約86,400行、約17 MB増える。250 ms周期なら約70 MB/日。Brokerが動いている間も、Pinkietなどの受信側には同じ件数が届く。
- Brokerに接続していれば、PUBACKのたびに行が消えるため定常状態のoutboxは数行に留まる。

初期版ではoutboxの容量上限と間引きを持たない。量を決めるのはセンサーの入力周期なので、Broker停止が長引く現場や受信側の保存量が問題になる現場では、ディスクの空き容量と`measurement` pipelineの入力周期を照らし、必要なら入力周期を下げる。
