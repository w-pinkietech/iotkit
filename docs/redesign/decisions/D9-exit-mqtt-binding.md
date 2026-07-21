# D9: 出口契約の第一バインディングは標準MQTT

Status: 確定、2026-07-18 Edge Node activation admission追加

## 決定

Edge NodeからIoTKit Edgeへの第一出口バインディングは、MQTT Brokerを使うMQTT 3.1.1 QoS 1とする。
IoTKitはBrokerやMQTT session implementationを自作しない。

```text
Edge Node durable outbox
  -> records topic (QoS 1)
  -> MQTT Broker
  -> IoTKit Edge
  -> raw records + accepted-through cursorを同一transactionでcommit
  -> accepted-through topic (QoS 1)
  -> Edge Node cursor advance / purge eligibility
```

MQTT PUBACKとapplication-levelの保管完了確認は異なる。

- **PUBACK**: brokerがMQTT publicationを受領したことだけを表す。
- **accepted-through**: IoTKit Edgeがraw recordと連続cursorを耐久保存した保管完了確認を表す。
- **purge権威**: Edge Nodeは相関を検証したaccepted-throughだけでcursorを進める。PUBACKだけでは進めない。

## Topic

Version 1のcustody namespace 2つ、現在状態複製1つ、Edge Node activation 2つは次のとおりである。

```text
iotkit/v1/edge-nodes/{edge_node_id}/records
iotkit/v1/edge-nodes/{edge_node_id}/accepted-through
iotkit/v1/edge-nodes/{edge_node_id}/descriptors
iotkit/v1/edge-nodes/{edge_node_id}/activation/request
iotkit/v1/edge-nodes/{edge_node_id}/activation/result
```

recordsとaccepted-throughはQoS 1、non-retainedとする。descriptorsはQoS 1、retainedのcomplete
current-state snapshotで、custody、cursor、activation権威、purge権威を持たない。activation/requestと
activation/resultはQoS 1、non-retainedとし、両端のdurable stateからapplication結果まで再送する。
Edge Nodeは自分のrecords/descriptors/activation resultだけをpublishし、自分のaccepted-through/activation request
だけをsubscribeできる。IoTKit Edgeは逆の権限を持つ。descriptorのdecode/保存失敗は、登録済みEdge Nodeの
raw受理とaccepted-throughを止めない。wildcard application publish、
device-to-device routing、`production`等のapplication固有topicはIoTKitの出口契約に含めない。

## Broker enrollmentとEdge Node activation

Broker credential/ACLを発行するenrollmentと、IoTKit Edgeが収集開始を承認するactivationを分離する。Brokerへ接続
できることはIoTKit Edge raw historyへの参加許可を意味しない。

- 未登録Edge Nodeはdescriptorだけを送信し、観測を`readings`へローカル保存できるがpublication logへ採番しない。
- IoTKit EdgeはdescriptorからEdge Nodeを`discovered`として表示し、adminのtyped operationで一意なactivation IDと
  exact `(edge_node_id, ledger_epoch)` grant、監査、command outboxを同一transactionへcommitする。
- Edge Nodeはcollectorと同じSQLite write serializationで境界を一度だけ確定し、将来のingest transactionだけを
  publicationへ入れる。publication log、AUTOINCREMENT sequence、target cursorが未使用でなければ初回
  activationを拒否し、remote cleanupやsequence巻き戻しを行わない。
- IoTKit Edgeはmatching activation resultをcommitして`active`になった後だけrecordsを受理する。状態・epoch検査は
  raw/cursor transaction内で行い、未登録recordsへackしない。
- activation前prefixは即時にpublication/query対象外となり、物理削除は固定境界を使う再開可能なEdge Node-local
  cleanupとする。activation result、通常再接続、credential更新は削除権威ではない。
- post-activation outboxは従来どおり、validated `accepted-through`だけでcursorとpurge eligibilityを進める。

## 配送と保管責任

- Edge Node outboxが再送の権威である。Broker sessionを正本にしない。
- deliveryはat-least-onceで、同一batchの再送は同じpublication identityと内容を使う。
- IoTKit Edgeは`(edge_node_id, ledger_epoch, pub_seq)`で冪等保存する。
- 同じrecord identityで異なる内容を受信した場合は保管競合(custody conflict)とし、cursorを進めない。
- IoTKit EdgeのSQL失敗、ENOSPC、corruption、commit前cancelではaccepted-throughをpublishしない。
- commit後のack消失は同一batch再送と冪等確認で回復する。
- 最初の実装はapplication ack待ちbatchを1つに制限する。PUBACKはこの窓を解放しない。
- accepted-throughは連続prefixだけを表し、gapを飛び越えない。
- 初回activation後の正式publicationは`pub_seq=1`から始まる。保存していないprefixを
  accepted-throughや別epochで偽装しない。

## Broker and availability

第一実装ではMosquitto等の既製MQTT BrokerをIoTKit Edge側へ配置する。Broker停止、IoTKit Edge停止、経路断の間もEdge Nodeは
ローカル収集を継続し、accepted-throughを受けていないoutboxを保持する。復旧後に再送して収束する。

Brokerの永続queueは追加の配送bufferとして使ってよいが、Edge Nodeの保管責任を代替しない。

## Superseded design

2026-07-13以前の「IoTKit EdgeのSQLite commit後にMQTT PUBACKを手動送出する内蔵Rust listener」は廃止した。
custom listener、manual PUBACK、独自keepalive/session/backpressure implementationは実装しない。
正式水位がapplication-level accepted-throughであるため、PUBACKを保管完了確認と同一化する必要はない。

## Deferred

- terminal noticeとgap修復protocol
- Broker federationと非預かりfan-out
- multi-Edge Node運用最適化
- HTTPSからMQTTへの既存target移行
- MQTT Broker HA
- deactivation/reactivation、IoTKit Edge間移動、Edge Node ID再利用
- 既存standalone outboxの自動adoptionとsame-epoch `stream_start_after`
- clone検知とIoTKit Edge restore generation fence

これらはペアリング済みBravePI Transmitter 1台 + 1 Edge Node + 1 IoTKit Edgeの実機縦切り後に、観測された必要性から決める。
