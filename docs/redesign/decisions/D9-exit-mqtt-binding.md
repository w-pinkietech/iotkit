# D9: 出口契約の第一バインディングは標準MQTT

Status: 確定、2026-07-15 descriptor current-state複製追加

## 決定

EdgeからSiteへの第一出口バインディングは、MQTT Brokerを使うMQTT 3.1.1 QoS 1とする。
IoTKitはBrokerやMQTT session implementationを自作しない。

```text
Edge durable outbox
  -> records topic (QoS 1)
  -> MQTT Broker
  -> IoTKit Site
  -> raw records + accepted-through cursorを同一transactionでcommit
  -> accepted-through topic (QoS 1)
  -> Edge cursor advance / purge eligibility
```

MQTT PUBACKとapplication-levelの保管完了確認は異なる。

- **PUBACK**: brokerがMQTT publicationを受領したことだけを表す。
- **accepted-through**: IoTKit Siteがraw recordと連続cursorを耐久保存した保管完了確認を表す。
- **purge権威**: Edgeは相関を検証したaccepted-throughだけでcursorを進める。PUBACKだけでは進めない。

## Topic

Version 1のcustody namespace 2つと、現在状態複製1つは次のとおりである。

```text
iotkit/v1/edge-nodes/{edge_node_id}/records
iotkit/v1/edge-nodes/{edge_node_id}/accepted-through
iotkit/v1/edge-nodes/{edge_node_id}/descriptors
```

recordsとaccepted-throughはQoS 1、non-retainedとする。descriptorsはQoS 1、retainedのcomplete
current-state snapshotで、custody、cursor、purge権威を持たない。Edgeは自分のrecords/descriptorsへだけ
publishし、自分のaccepted-throughだけをsubscribeできる。IoTKit Siteは逆の権限を持つ。descriptorの
decode/保存失敗はraw受理とaccepted-throughを止めない。wildcard application publish、
device-to-device routing、`production`等のapplication固有topicはIoTKitの出口契約に含めない。

## 配送と保管責任

- Edge outboxが再送の権威である。Broker sessionを正本にしない。
- deliveryはat-least-onceで、同一batchの再送は同じpublication identityと内容を使う。
- Siteは`(edge_node_id, ledger_epoch, pub_seq)`で冪等保存する。
- 同じrecord identityで異なる内容を受信した場合は保管競合(custody conflict)とし、cursorを進めない。
- SiteのSQL失敗、ENOSPC、corruption、commit前cancelではaccepted-throughをpublishしない。
- commit後のack消失は同一batch再送と冪等確認で回復する。
- 最初の実装はapplication ack待ちbatchを1つに制限する。PUBACKはこの窓を解放しない。
- accepted-throughは連続prefixだけを表し、gapを飛び越えない。

## Broker and availability

第一実装ではMosquitto等の既製MQTT BrokerをSite側へ配置する。Broker停止、Site停止、経路断の間もEdgeは
ローカル収集を継続し、accepted-throughを受けていないoutboxを保持する。復旧後に再送して収束する。

Brokerの永続queueは追加の配送bufferとして使ってよいが、Edgeの保管責任を代替しない。

## Superseded design

2026-07-13以前の「SiteのSQLite commit後にMQTT PUBACKを手動送出する内蔵Rust listener」は廃止した。
custom listener、manual PUBACK、独自keepalive/session/backpressure implementationは実装しない。
正式水位がapplication-level accepted-throughであるため、PUBACKを保管完了確認と同一化する必要はない。

## Deferred

- terminal noticeとgap修復protocol
- Broker federationと非預かりfan-out
- multi-Edge運用最適化
- HTTPSからMQTTへの既存target移行
- MQTT Broker HA

これらはペアリング済みBravePI Transmitter 1台 + 1 Edge Node + 1 Siteの実機縦切り後に、観測された必要性から決める。
