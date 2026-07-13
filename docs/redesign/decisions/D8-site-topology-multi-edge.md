# D8: Site topology and multi-Edge boundary

Status: 確定、2026-07-13簡素化改訂

## Topology

IoTKitは2つの配置を認める。

### Standalone

- 1台のRaspberry Piが完全なIoTKit Edgeを動かす。
- Edgeはローカル収集、正規化、SQLite上の耐久buffer、outbox、再送、queryを持つ。
- IoTKit Siteや上流接続は任意である。

### Site-managed

- 2台以上のEdge Nodeを置く場合も、各Piは完全なEdge Nodeである。
- 代表Pi、親Edge Node、中央collectorは置かない。
- IoTKit SiteはMQTT Broker、Archival Store、Edge Nodeごとのcursor、site-level query、application接続・export境界を提供する。
- IoTKit Siteはsensor busを読まず、Edgeのcollectorやregistryの権威を奪わない。
- cloudは任意の上位層であり、Siteによる保管責任引受の必須条件ではない。

最初の実機縦切りは、ペアリング済みBravePI Transmitter 1台 + 1 Edge Node + 1 Siteで通信と保管責任の
引き渡しを証明する。これはmulti-Edge運用UI、fleet管理、
一括enrollmentを実装する意味ではない。

## Custody roles

- **IoTKit Edge**: 観測を耐久保存し、保管完了確認までoutboxを保持する。
- **MQTT Broker**: QoS 1 transportを提供する。PUBACKはpurge権威ではない。
- **Site Archival Store**: raw canonical recordsと連続cursorを同一transactionで保存し、
  application-level accepted-throughを返す。
- **Application consumer**: YokaKit、dashboard、analytics等。Siteのcanonical streamを自分のdomainへ投影する。
  通常はnon-custodialであり、その業務処理結果はEdge purgeを許可しない。

Site query projectionやapplication exportが壊れても、Archival Storeが引き受けたraw recordの保管責任とEdge cursorを
巻き戻したり進めたりしない。

## Identity and ordering

Site側のglobal record identityは次である。

```text
(edge_node_id, ledger_epoch, publication_seq)
```

- `edge_node_id`は各Edge Nodeが初回構成時に生成し、共有imageへ焼き込まない。
- Edge Nodeごとのpublication orderはあるが、複数Edge Node間の全順序は定義しない。
- Site全体の配送状態は単一cursorでなくEdge Nodeごとのwatermark vectorで表す。
- 同じglobal identityに異なる内容が到着した場合は上書きせず保管競合(custody conflict)とする。
- deliveryはat-least-onceであり、Site保存とapplication projectionは冪等にする。

## Failure behavior

- Site、Broker、overlay停止中も各Edge Nodeはローカル収集を継続し、未ack outboxを保持する。
- 復旧後は各Edge Nodeが独立に再送し、accepted-throughへ収束する。
- SiteのSQL失敗、ENOSPC、corruptionではapplication ackを返さない。
- 一部Edge Nodeだけ不達の場合、Siteは欠けたEdge Nodeを明示し、site集計を完全値として表示しない。
- Edgeの保持容量を超える長期障害はR17の明示的degradation/data-loss規則に従う。無音削除しない。

## YokaKit boundary

YokaKitはIoTKitのcanonical recordを消費する別applicationである。設備、工程、製品、作業者、生産状態、
OEE、alarm文言、UIはYokaKitが所有する。IoTKitは`production`、`gantt-chart`等のYokaKit固有topicや
table語彙をR10へ入れない。Siteのapplication exportは保存済みseriesのrouting・projectionを担うが、
`production`等の業務意味を解釈しない。

## Deferred

次は最初の実機縦切りの完了条件ではない。

- fleet enrollmentとduplicate Edge Node recovery
- Site Consoleと統合承認UI
- credential rotation/decommission automation
- Site backup/restore、generation anchor、archive repair orchestration
- multi-Edge capacity planningとpartial-partition UI
- site-wide snapshot、cloud federation、HA
- YokaKit互換projection

必要になった項目は実機運用の証拠と明確な失敗経路を基に別途決める。将来候補を現在のMVPへ戻さない。
