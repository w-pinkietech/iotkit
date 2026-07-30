---
type: Runbook
title: "このPCでIoTKitを試す"
description: "Loopback限定のIoTKit試用profileを起動、確認、停止、初期化する手順です。"
language: ja
translation_key: operations.trial-profile
status: draft
revision: 1
---

# このPCでIoTKitを試す

現場導入の判断を始める前に、実際のIoTKit収集loopを確認するためのprofileです。
一つのLinux host上でIoTKit Edge Node、標準Mosquitto Broker、IoTKit Edgeを実行し、
全network listenerをIPv4 loopbackに限定します。生成sampleは通常のInput Adapterと
保管責任contractを通り、Console mockやDB seedではありません。

このprofileは評価専用です。TLS、backup、実sensor、PostgreSQL、HA、現場向けupdateは
設定しません。

## 必要なもの

- GitとPython 3.11以降を使用できる対応Linux host。
- `docker compose` commandを含むDocker Engine。
- local TCP port 8080と18883の空き。

## 起動

cleanなrepository cloneで実行します。

```bash
./scripts/iotkit trial validate
./scripts/iotkit trial up
```

表示に従い、12文字以上128文字以下の試用管理者passwordを決めます。Launcherは
passwordを`iotkit.toml`、command argument、出力へ書きません。生成credentialとDBは
repository外の`${XDG_DATA_HOME:-$HOME/.local/share}/iotkit/trial`へowner-only権限で
保存します。

`http://127.0.0.1:8080`を開き、login ID `admin`と決めたpasswordでログインします。
黄色の**お試し環境**表示が常に見えることを確認してください。

## 最短の確認手順

1. **概要**で収集ノードが1台検出されていることを確認する。
2. **機器管理**で収集ノードを選び、有効化する。
3. 試用照度sensorを開き、案内された表示設定を完了する。
4. **センサー一覧**で照度値が変化することを確認する。
5. **受信履歴**で行が増えることを確認する。

有効化は実際の保管責任contractの一部なので、試用でも明示操作として残しています。
停止と再起動ではDBを削除しません。

```bash
./scripts/iotkit trial status
./scripts/iotkit trial down
./scripts/iotkit trial up
```

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
console_port = 18080
broker_port = 18884
sample_interval_ms = 1000
```

`console_bind`と`broker_bind`に指定できるのはIPv4 loopback addressだけです。
未知のkey、version、profile、loopback以外のbindは拒否します。
