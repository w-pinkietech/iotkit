---
type: Runbook
title: "このPCでIoTKitを試す"
description: "Loopback限定のIoTKit試用profileを起動、確認、停止、初期化する手順です。"
language: ja
translation_key: operations.trial-profile
status: draft
revision: 6
---

# このPCでIoTKitを試す

現場導入の判断を始める前に、IoTKitが実際にObservationを公開する様子を確認するためのprofileです。
一つのLinux host上でIoTKit Edge Nodeと標準のMosquitto Brokerを実行し、networkのlistenerを
IPv4 loopbackに限定します。生成したsampleは通常のInput Adapterとpipelineを通って
[MQTT Output Adapter契約 v1](../contracts/mqtt-output-adapter-v1.md)のtopicへ公開され、
`mosquitto_sub`がそれを購読します。Console mockやDB seedではありません。

このprofileは評価専用です。TLS、実sensor、現場向けupdateは設定しません。

## 必要なもの

- GitとPython 3.14以降を使用できる対応Linux host。
- `docker compose` commandを含むDocker Engine。
- local TCP port 18883の空き。

## 起動

cleanなrepository cloneで実行します。

```bash
./scripts/iotkit trial validate
./scripts/iotkit trial up
```

初回の`up`はEdge Nodeのimageをbuildするため、host性能により数分かかります。途中で中断した
場合は、同じ`up`をもう一度実行すると初期化をやり直します。生成したcredentialとDBは
repository外の`${XDG_DATA_HOME:-$HOME/.local/share}/iotkit/trial`へowner-only権限で保存します。

`up`は、Edge Nodeの起動後に次の3本のpipeline定義をimportします。edge-node-idは`trial`です。

| pipeline-id | kind | 入力 |
|---|---|---|
| `sample-illuminance` | `measurement` | 試用照度（三角波、120〜200 lx、入力ごとに公開） |
| `sample-contact` | `state` | 試用接点状態（矩形波）の二値化 |
| `sample-cycles` | `accumulated-count` | 同じ接点状態の立ち上がり回数 |

## 最短の確認手順

```bash
./scripts/iotkit trial watch
```

`watch`はBroker内の`mosquitto_sub`を読み取り専用のaccountで動かし、届いたtopic、retainフラグ、
payloadを1行ずつ表示します。Ctrl-Cで終了します。次を確認してください。

1. `iotkit/v1/edge-node/trial/status`に`"value":"online"`と`"faults":[]`のheartbeatが届く。
2. `.../observation/sample-illuminance/measurement`の`value`が8ずつ増減し、`sequence`が1ずつ増える。
3. `.../observation/sample-contact/state`の`value`が`true`と`false`を交互に繰り返す。
4. `.../observation/sample-cycles/accumulated-count`の`value`が`0`から始まり、`state`が`true`に
   なるたびに1増える。
5. `watch`を再実行すると、最初にretainされた最新値（retainフラグ`1`）が各topicにつき1件届く。

停止と再起動ではDBを削除しません。再起動後もseriesとsequenceは続きます。

```bash
./scripts/iotkit trial status
./scripts/iotkit trial down
./scripts/iotkit trial up
```

`trial down`では、Edge Nodeに15秒のgraceful-stop windowを与えます。Edge Nodeは時刻付きの
`offline`をstatusへ公開してから切断し、未送信のObservationはoutboxに残って次の`up`で届きます。

## 初期化

初期化は認識済みの試用state directoryだけを削除し、明示的なdata-loss確認を要求します。

```bash
./scripts/iotkit trial reset --confirm-trial-data-loss
```

現場へ導入するときは、ここで試用を止めて
[導入と復旧](installation-and-recovery.md)へ進みます。試用stateを現場環境へ昇格は
できません。

## Portを変更する場合

repository直下の2行だけで起動できます。既定portが別processと競合するときだけ、
必要な設定を追加します。

```toml
config_version = 1
profile = "trial"

[trial]
broker_port = 18884
sample_interval_ms = 1000
```

`broker_bind`に指定できるのはIPv4 loopback addressだけです。
未知のkey、version、profile、loopback以外のbindは拒否します。
