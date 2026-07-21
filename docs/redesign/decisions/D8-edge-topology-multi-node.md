# D8: IoTKit Edge topology and multi-Edge Node boundary

Status: 確定、2026-07-18 Edge Node activation境界追記

## Topology

IoTKitは2つの配置を認める。

### Standalone

- IoTKit EdgeへactivationされていないIoTKit Edge Nodeが、単独で収集・保全する。
- Edge Nodeはローカル収集、正規化、SQLite上の耐久buffer、outbox、再送、queryを持つ。
- IoTKit Edgeへの接続を開始した時点でEdge-connectedへ移る。

### Edge-connected

- 1台以上のEdge NodeをIoTKit Edgeへactivationして接続する。台数に関係なく、各Piは完全なEdge Nodeである。
- 代表Pi、親Edge Node、中央collectorは置かない。
- IoTKit EdgeはArchival Store、Edge Nodeごとのcursor、Edge Node-scoped query、設定可能な
  センサー意味付け、application接続・export境界を提供する。MQTT Brokerは独立したtransport依存である。
- IoTKit Edgeはsensor busを読まず、Edge Nodeのcollectorやregistryの権威を奪わない。
- Fleet layerは任意の上位層であり、IoTKit Edgeによる保管責任引受の必須条件ではない。

最初の実機縦切りは、ペアリング済みBravePI Transmitter 1台 + 1 Edge Node + 1 IoTKit Edgeで通信と保管責任の
引き渡しを証明する。これはmulti-Edge Node運用UI、fleet管理、
一括enrollmentを実装する意味ではない。

Edge-connected構成では、各Edge NodeのBroker enrollmentとEdge Node activationを分離する。Broker credentialとACLは
導入担当者が各hostのlocal手順で配布する。IoTKit Consoleはdescriptorで発見したEdge Nodeをadminが一度だけ
activationし、そのexact ledger incarnationについて将来のpublicationを受け入れる。ConsoleはBroker設定、
credential、ACLを作成・変更しない。

## Custody roles

- **IoTKit Edge Node**: 観測を耐久保存し、保管完了確認までoutboxを保持する。
- **MQTT Broker**: QoS 1 transportを提供する。PUBACKはpurge権威ではない。
- **IoTKit Edge Archival Store**: raw canonical recordsと連続cursorを同一transactionで保存し、
  application-level accepted-throughを返す。
- **Application consumer**: YokaKit、dashboard、analytics等。IoTKit Edgeのcanonical streamを自分のdomainへ投影する。
  通常はnon-custodialであり、その業務処理結果はEdge Node purgeを許可しない。

IoTKit Edge query projectionやapplication exportが壊れても、Archival Storeが引き受けたraw recordの保管責任とEdge Node cursorを
巻き戻したり進めたりしない。

activation前のローカル確認値はpublication outboxへ採番されず、IoTKit Edge Archival Storeが引き受けるcanonical
publicationではない。activation境界より後にpublicationへ入ったrecordだけが上記custodyの対象になる。

## Identity and ordering

IoTKit Edge側のglobal record identityは次である。

```text
(edge_node_id, ledger_epoch, publication_seq)
```

- `edge_node_id`は各Edge Nodeが初回構成時に生成し、共有imageへ焼き込まない。
- Edge Nodeごとのpublication orderはあるが、複数Edge Node間の全順序は定義しない。
- IoTKit Edge全体の配送状態は単一cursorでなくEdge Nodeごとのwatermark vectorで表す。
- 同じglobal identityに異なる内容が到着した場合は上書きせず保管競合(custody conflict)とする。
- deliveryはat-least-onceであり、IoTKit Edge保存とapplication projectionは冪等にする。

## Failure behavior

- IoTKit Edge、Broker、network経路停止中も各Edge Nodeはローカル収集を継続し、未ack outboxを保持する。
- 未登録Edge Nodeはboundedなローカル確認値だけを保持し、IoTKit Edge向けoutboxを作らない。
- 復旧後は各Edge Nodeが独立に再送し、accepted-throughへ収束する。
- IoTKit EdgeのSQL失敗、ENOSPC、corruptionではapplication ackを返さない。
- 一部Edge Nodeだけ不達の場合、IoTKit Edgeは欠けたEdge Nodeを明示し、全体集計を完全値として表示しない。
- Edge Nodeの保持容量を超える長期障害はR17の明示的degradation/data-loss規則に従う。無音削除しない。

## YokaKit boundary

YokaKitはIoTKitのcanonical recordまたはIoTKit Edgeで意味付けされた出力を消費する別applicationである。
IoTKit Edgeは保存済みseriesを`production_pulse`等の設定可能なセンサー意味へ対応付け、routing・projectionする。
一方、設備・工程・製品・作業者のmaster、生産実績、OEE、alarm文言、UIはYokaKitが所有する。
YokaKit固有topic/table語彙はR10へ入れない。

## Deferred

次は最初の実機縦切りの完了条件ではない。

- fleet enrollmentとduplicate Edge Node recovery
- credential rotation/decommission automation
- deactivation/reactivation、IoTKit Edge間移動、既存standalone outboxの自動adoption
- IoTKit Edge backup/restore、generation anchor、archive repair orchestration
- multi-Edge Node capacity planningとpartial-partition UI
- Edge Node-wide snapshot、cloud federation、HA
- YokaKit互換projection

必要になった項目は実機運用の証拠と明確な失敗経路を基に別途決める。将来候補を現在のMVPへ戻さない。
