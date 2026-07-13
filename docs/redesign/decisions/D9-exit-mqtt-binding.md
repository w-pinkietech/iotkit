# D9: 出口契約の第一バインディングは標準MQTT

Status: 確定、2026-07-13簡素化改訂

## 決定

GatewayからSiteへの第一出口バインディングは、標準MQTT brokerを使うMQTT 3.1.1 QoS 1とする。
IoTKitはbrokerやMQTT session implementationを自作しない。

```text
Gateway durable outbox
  -> records topic (QoS 1)
  -> standard MQTT broker
  -> Site Server
  -> raw records + accepted-through cursorを同一transactionでcommit
  -> accepted-through topic (QoS 1)
  -> Gateway cursor advance / purge eligibility
```

MQTT PUBACKとapplication-levelの保管完了確認は異なる。

- **PUBACK**: brokerがMQTT publicationを受領したことだけを表す。
- **accepted-through**: Site Serverがraw recordと連続cursorを耐久保存した保管完了確認を表す。
- **purge権威**: Gatewayは相関を検証したaccepted-throughだけでcursorを進める。PUBACKだけでは進めない。

## Topic

Version 1の最小namespaceは次の2つである。

```text
iotkit/v1/gateways/{gateway_identity}/records
iotkit/v1/gateways/{gateway_identity}/accepted-through
```

どちらもQoS 1、non-retainedとする。Gatewayは自分のrecordsへだけpublishし、自分の
accepted-throughだけをsubscribeできる。Site Serverは逆の権限を持つ。wildcard application publish、
device-to-device routing、`production`等のapplication固有topicはIoTKitの出口契約に含めない。

## 配送と保管責任

- Gateway outboxが再送の権威である。broker sessionを正本にしない。
- deliveryはat-least-onceで、同一batchの再送は同じpublication identityと内容を使う。
- Siteは`(gateway_identity, ledger_epoch, pub_seq)`で冪等保存する。
- 同じrecord identityで異なる内容を受信した場合は保管競合(custody conflict)とし、cursorを進めない。
- SiteのSQL失敗、ENOSPC、corruption、commit前cancelではaccepted-throughをpublishしない。
- commit後のack消失は同一batch再送と冪等確認で回復する。
- 最初の実装はapplication ack待ちbatchを1つに制限する。PUBACKはこの窓を解放しない。
- accepted-throughは連続prefixだけを表し、gapを飛び越えない。

## Broker and availability

第一実装ではMosquitto等の既製brokerをSite側へ配置する。broker停止、Site停止、経路断の間もGatewayは
ローカル収集を継続し、accepted-throughを受けていないoutboxを保持する。復旧後に再送して収束する。

brokerの永続queueは追加の配送bufferとして使ってよいが、Gatewayの保管責任を代替しない。

## Superseded design

2026-07-13以前の「SiteのSQLite commit後にMQTT PUBACKを手動送出する内蔵Rust listener」は廃止した。
custom listener、manual PUBACK、独自keepalive/session/backpressure implementationは実装しない。
正式水位がapplication-level accepted-throughであるため、PUBACKを保管完了確認と同一化する必要はない。

## Deferred

- terminal noticeとgap修復protocol
- broker federationと非預かりfan-out
- multi-Gateway運用最適化
- HTTPSからMQTTへの既存target移行
- broker HA

これらはペアリング済みBravePI温度センサー1台 + 1 Gateway + 1 Siteの実機縦切り後に、観測された必要性から決める。
