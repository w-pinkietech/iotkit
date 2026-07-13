# D8: Site topology and multi-Gateway boundary

Status: 確定、2026-07-13簡素化改訂

## Topology

IoTKitは2つの配置を認める。

### Standalone

- 1台のRaspberry Piが完全なGatewayを動かす。
- Gatewayはローカル収集、SQLite custody、outbox、queryを持つ。
- Site Serverや上流接続は任意である。

### Site-managed

- 2台以上のGatewayを置く場合も、各Piは完全なGatewayである。
- 代表Pi、親Gateway、中央collectorは置かない。
- Site Serverは標準MQTT broker、Archival Store、site-level query/application接続点を提供する。
- Site Serverはsensor busを読まず、Gateway collectorやregistryの権威を奪わない。
- cloudは任意の上位層であり、Site custodyの必須条件ではない。

最初の実機縦切りは1 Gateway + 1 Siteで通信とcustodyを証明する。これはmulti-Gateway運用UI、fleet管理、
一括enrollmentを実装する意味ではない。

## Custody roles

- **Gateway**: 観測を耐久保存し、application custody ackまでoutboxを保持する。
- **MQTT broker**: QoS 1 transportを提供する。PUBACKはpurge権威ではない。
- **Site Archival Store**: raw canonical recordsと連続cursorを同一transactionで保存し、
  application-level accepted-throughを返す。
- **Application consumer**: YokaKit、dashboard、analytics等。Siteのcanonical streamを自分のdomainへ投影する。
  通常はnon-custodialであり、その業務処理結果はGateway purgeを許可しない。

Site query projectionやapplication projectionが壊れても、Archival Storeのraw custodyとGateway cursorを
巻き戻したり進めたりしない。

## Identity and ordering

Site側のglobal record identityは次である。

```text
(gateway_identity, ledger_epoch, publication_seq)
```

- `gateway_identity`は各Gatewayが初回構成時に生成し、共有imageへ焼き込まない。
- Gatewayごとのpublication orderはあるが、複数Gateway間の全順序は定義しない。
- Site全体の配送状態は単一cursorでなくGatewayごとのwatermark vectorで表す。
- 同じglobal identityに異なる内容が到着した場合は上書きせずcustody conflictとする。
- deliveryはat-least-onceであり、Site保存とapplication projectionは冪等にする。

## Failure behavior

- Site、broker、overlay停止中も各Gatewayはローカル収集を継続し、未ack outboxを保持する。
- 復旧後は各Gatewayが独立に再送し、accepted-throughへ収束する。
- SiteのSQL失敗、ENOSPC、corruptionではapplication ackを返さない。
- 一部Gatewayだけ不達の場合、Siteは欠けたGatewayを明示し、site集計を完全値として表示しない。
- Gatewayの保持容量を超える長期障害はR17の明示的degradation/data-loss規則に従う。無音削除しない。

## YokaKit boundary

YokaKitはIoTKitのcanonical recordを消費する別applicationである。設備、工程、製品、作業者、生産状態、
OEE、alarm文言、UIはYokaKitが所有する。IoTKitは`production`、`gantt-chart`等のYokaKit固有topicや
table語彙をR10へ入れない。

## Deferred

次は最初の実機縦切りの完了条件ではない。

- fleet enrollmentとduplicate Gateway recovery
- Site Consoleと統合承認UI
- credential rotation/decommission automation
- Site backup/restore、generation anchor、archive repair orchestration
- multi-Gateway capacity planningとpartial-partition UI
- site-wide snapshot、cloud federation、HA
- YokaKit互換projection

必要になった項目は実機運用の証拠と明確な失敗経路を基に別途決める。将来候補を現在のMVPへ戻さない。
