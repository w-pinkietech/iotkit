---
type: Contract Overview
title: "IoTKit Input Adapter契約 v1の概要"
description: "センサー固有実装を汎用Edge Node hostへ組み込む契約の入口です。"
language: ja
translation_key: contracts.input-adapter-v1
status: stable
revision: 1
---

# IoTKit Input Adapter契約 v1の概要

この文書は境界を理解するための概要です。型、lifecycle、設定、conformanceの正式契約は、同じGit revisionの`docs/input-adapter-contract.md`とexported host API、testkitの契約成果物setにあります。

Input Adapterは、特定のベンダー、通信方式、機器モデルをIoTKitへ接続する説明責任単位です。IoTKit coreへベンダー固有語彙を漏らさず、観測を共通取り込み契約へ渡します。

責務は次のように分離します。

* transport backendはserial、I2C、GPIOなどのraw I/Oだけを扱う。
* device driverはprotocol、register、検出、初期化、データシート由来の物理量変換を扱う。
* adapter runtime/compositionはdriverの実行、lifecycle、measurement keyとchannelへの写像を扱う。
* ingest clientはEnvelope、ID採番、送信、Ack、再送を扱う。
* Edge Node hostは設定権限、principal作成、series解決、保存、再起動方針、health集約を所有する。

Adapter type、設定されたinstance、diagnostic source、認証principal、観測subject、IoTKit system identityは別のidentityです。送信するEnvelopeのsourceがhostからbindされたsourceと一致しない場合は受理しません。

Raspberry Pi 4B/5などのhost platformは能力検査に使いますが、Adapter、source、device identityへ混ぜません。BravePIは一実装例であり、同じhost contractへ別のAdapterとdriverを追加できます。

Adapterはstorage、custody cursor、semantic rule、Output Adapter、外部Broker credentialを所有しません。
