---
type: Contract
title: "IoTKit MQTT Output Adapter契約 v1"
description: "IoTKit端末がObservationとstatusを標準MQTT Brokerへ公開するときのtopic、payload、配信、consumerの義務を定義します。"
language: ja
translation_key: contracts.mqtt-output-adapter-v1
status: draft
revision: 1
---

# IoTKit MQTT Output Adapter契約 v1

状態: 契約を固定した段階。IoTKit本体からの公開は [#232](https://github.com/w-pinkietech/iotkit/issues/232) の子Issueで実装する。

## 1. 目的

IoTKit端末は、センサー入力を校正、二値化、ヒステリシス、デバウンス、累積カウントでObservationへ変換し、本契約のtopicとpayloadで標準のMQTT Brokerへ公開する。
Pinkietなどの業務アプリケーションはBrokerを購読し、Observationを自分のドメインへ対応付ける。

```text
sensor -> Input Adapter -> pipeline -> MQTT Output Adapter v1 -> MQTT Broker -> consumer
         |<-------------------- IoTKit（ハード1台につき1インスタンス）------------------->|
```

本契約はプロトコル別の契約であり、業務アプリケーション別ではない。
production、alarm、Ganttなどの業務用語はtopicにもpayloadにも現れない。
Observationそのもののモデル（kind、series、sequence、timestamp）は[製品モデル](../concepts/product-model.md)に置き、本契約はそれをMQTTへ写す。

## 2. 識別子

edge-node-idとpipeline-idは次の正規表現に一致し、UTF-8で1〜64バイトとする。

```text
^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$
```

- edge-node-idはIoTKit端末を識別し、起動設定で与える。1つのBroker namespaceの中で一意とする。
- pipeline-idは端末内の処理pipelineを識別する設定上のIDであり、物理センサーのIDではない。

初期版では、Broker namespace 1つをconsumer側のsite 1つに割り当てる。topicにEnterprise、Site、工場のIDは含めない。

## 3. Observation topic

```text
iotkit/v1/edge-node/{edge-node-id}/observation/{pipeline-id}/{kind-key}
```

kind-keyは`measurement`、`accumulated-count`、`state`のいずれかで、pipelineのkindと一致する。
pipeline 1つにつきtopicは1つである。

例：

```text
iotkit/v1/edge-node/rpi1/observation/press-01-temperature/measurement
iotkit/v1/edge-node/rpi1/observation/press-01-cycle-count/accumulated-count
iotkit/v1/edge-node/rpi1/observation/press-01-temperature-high/state
```

consumerは`iotkit/v1/edge-node/+/observation/+/+`で購読し、topicの各段からedge-node-id、pipeline-id、kindを得る。

## 4. Observation payload

```json
{
  "series_id": "018f5f83-7a2b-7c61-a729-6af238f558e0",
  "sequence": 42,
  "timestamp": 1784190000123,
  "value": 1250
}
```

| field | 型 | 意味 |
|---|---|---|
| `series_id` | 文字列（UTF-8で1〜64バイト） | 同じpipeline出力の連続した世代。等価比較にだけ使う不透明な文字列で、UUIDであることやversionをconsumerは検証しない |
| `sequence` | 整数（1以上、2^53−1以下） | series内で1から始まり、公開ごとに1増える。再送では増えない |
| `timestamp` | 整数（Unix epoch ms） | その出力を確定させた入力をIoTKitが受信した実時刻。debounceで確定した場合はdebounceを満了させた入力の受信時刻 |
| `value` | kindによる | 下表 |

| kind-key | `value` | 補足 |
|---|---|---|
| `measurement` | JSON number | 整数と小数を含む。単位はpayloadに含めず、pipeline設定とconsumer側の登録で管理する |
| `accumulated-count` | 0以上の整数（2^53−1以下） | pipelineが算出した累積値。series内で単調に増える |
| `state` | boolean | 二値化、ヒステリシス、デバウンス後の現在状態 |

fieldはこの4つだけであり、他のfieldは追加しない。
IoTKitはpayloadを、この順のkeyを持つ空白なしのJSON（`{"series_id":…,"sequence":…,"timestamp":…,"value":…}`）として送る。producerのconformance testはこのbytesをfixtureと比較する。
consumerはpayloadをJSONとして解釈し、keyの順序や空白に依存しない。

`sequence`は単調だが、`timestamp`は端末の時計補正で逆行することがある。順序の判定と重複排除には`sequence`を使う。
端末はNTP同期を必須とする。

## 5. seriesの規則

- 表示名の変更、閾値やデバウンスなど調整項目の変更、MQTT Broker設定の変更、通常のプロセス再起動、MQTT再接続ではseriesを変えない。
- pipelineの構造項目（kind、入力、trigger、単位）の変更、明示的な累積カウンタのリセット、pipeline定義のimport、SQLiteの状態喪失など、連続性を保証できない場合は新しいseriesを開始する。
- `accumulated-count`の新しいseriesは、開始時点で`sequence = 1, value = 0`を新しいseries_idで即時公開する。Brokerがretainしている値は常に現在のseriesを表し、consumerは最初の増分を数え損ねない。
- `state`と`measurement`は最初の入力で初期値を公開する。
- `accumulated-count`が2^53−1に達した場合、IoTKitはカウントを停止して端末のエラーとして表示する。自動rolloverはない。

## 6. 配信と責任境界

- MQTT 3.1.1、`clean_session = true`。IoTKitは自分のoutboxを唯一の正とし、再接続後はoutboxから再送する。
- すべてのObservationはQoS 1、retain有効で公開する。retainは履歴ではなく、Brokerにtopicごとの最新値を保持させるために使う。
- in-flightは1件で、outboxは挿入順に送る。したがって重複配信は直前のpublicationの再送に限られる。
- PUBACKはIoTKitの送信責任の境界である。PUBACK後、IoTKitはそのpublicationをoutboxから削除する。consumerのDB保存や業務処理の完了は保証しない。application-level ACKのtopicはない。
- 再接続の直後、IoTKitはstatusを先に公開し、続いてoutboxを再送する。

Broker停止中のpublicationはIoTKitのoutboxに残り、復旧後に同じtopicとpayloadで届く。
IoTKit側で入力を永続化できない間の入力は破棄され、statusが`degraded`になる（7節）。

## 7. status topic

```text
iotkit/v1/edge-node/{edge-node-id}/status
```

payload：

```json
{
  "timestamp": 1784190000123,
  "value": "online"
}
```

`value`は次の3値。

| value | 意味 | Observationへの影響 |
|---|---|---|
| `online` | MQTTに接続しており、入力を永続化できている | なし |
| `degraded` | MQTTに接続しており保存済みのoutboxは送信しているが、新しい入力を永続化できず破棄している | この区間の`accumulated-count`の増分は失われ、復旧後も届かない。consumerは欠測として記録する |
| `offline` | MQTTに接続していない | この区間のObservationはoutboxに保全され、復旧後に届く |

- heartbeatは定期的に公開する`online`または`degraded`である。接続確立の直後に即時公開し、以後は`heartbeat_interval`（既定60秒、5秒〜1時間）ごとにQoS 1、retain有効で公開する。
- `online`と`degraded`の切り替わりはheartbeatの間隔を待たずに即時公開する。
- 正常終了時、IoTKitは時刻付きの`offline`をQoS 1、retain有効で公開し、PUBACKを最大2秒待って切断する。
- 異常切断時はMQTTのWill（QoS 1、retain有効）によりBrokerが次を公開する。`timestamp`が`null`なのは、IoTKit自身が切断時刻を観測していないためである。

```json
{
  "timestamp": null,
  "value": "offline"
}
```

## 8. pipelineの削除

pipelineを削除したとき、IoTKitはそのObservation topicへ長さ0のpayloadをretain有効で公開する。
Brokerはそのtopicのretain値を消し、以後に購読するconsumerには何も届かない。
購読中のconsumerには長さ0のpayloadが届く。これは不正なJSONではなく、「この入力は利用できなくなった」という確定した事実である。

## 9. consumerの義務と前提

- `series_id`ごとに受信済み`sequence`の最大値を保持し、それ以下の`sequence`を重複として破棄してよい。`series_id`が変わったら最大値を捨て、新しいseriesを受け入れる。seriesの変更は異常ではなく基準値の更新である。
- `sequence`の欠番は異常ではない。retainは最新値しか残さないため、consumerの切断中に公開された中間の値は届かない。`accumulated-count`は累積値なので最新値で追従できる。すべてのObservationを漏れなく受けたいconsumerはBrokerとの間でpersistent sessionを使う。
- subscribe直後に届くretainされた最新値を初期値として扱う。`accumulated-count`の最初に受け取った値は基準値であり、その累積の総数を業務実績に一括で加算しない。
- 同じseriesの中で`accumulated-count`が減った場合は異常として扱い、黙って基準値を置き換えない。
- 長さ0のpayloadをpipelineの削除として扱う（8節）。
- `degraded`の区間を欠測として記録し、`offline`と区別する（7節）。
- heartbeatの`timestamp`と受信時刻の差から、端末の時計のずれを検出できる。時刻差だけを理由にObservationを破棄しない。

## 10. 契約の成果物

| 成果物 | path |
|---|---|
| Observation payloadのJSON Schema | `testdata/observation/v1/observation.schema.json` |
| status payloadのJSON Schema | `testdata/observation/v1/status.schema.json` |
| fixtureの形式 | `testdata/observation/v1/fixture.schema.json` |
| 公開のfixture（topic、QoS、retain、payload bytes） | `testdata/observation/v1/*.json` |
| 拒否されるべき例 | `testdata/observation/v1/invalid/*.json` |
| Schema適合と正規形の検査 | `node scripts/check-observation-fixtures.mjs` |
| 受信側の照合（Brokerへ流して購読側で確認） | `scripts/test-observation-consumer.sh` |

fixtureはproducerとconsumerの両方の正である。IoTKitのconformance testは公開するtopicとbytesをfixtureと比較し、consumerのconformance testとシミュレータはfixtureからpayloadを生成する。
文書、Schema、fixture、検査のどれかが食い違えば契約の欠陥として扱い、黙って片方に合わせない。

## 11. 未決定の事項

- `measurement`の公開頻度（入力ごと、変化時のみ、最小間隔のいずれか）。#232 の子Issueで決め、本契約のrevisionを上げる。
- pipeline削除の長さ0 payloadをoutboxに入れて到達を保証するか。Broker切断中に削除した場合の扱いとして、同じく子Issueで決める。
