---
type: Contract Overview
title: "IoTKit Output Adapter契約 v1の概要"
description: "汎用Observationを外部アプリケーション固有のMQTT publicationへ変換する契約の入口です。"
language: ja
translation_key: contracts.output-adapter-v1
status: stable
revision: 1
---

# IoTKit Output Adapter契約 v1の概要

この文書は境界を理解するための概要です。型、設定schema、error、bindingの正式契約は、同じGit revisionの`docs/output-adapter-contract.md`とexported Go types、共有fixtureの契約成果物setにあります。

Output Adapterは、IoTKit Edgeの汎用Observationと版管理されたroute設定を入力し、一つの外部アプリケーション向けMQTT publicationを決定的に返すin-process transformerです。

汎用Observationのkindは次のとおりです。

| kind | value | 意味 |
|---|---|---|
| `numeric` | 有限JSON number | 補正または変換済み数値 |
| `boolean` | JSON boolean | 汎用ON/OFF状態 |
| `cumulative_value` | 0以上のJSON integer | 起点以降の累積値 |
| `alarm` | JSON boolean | 発報または解除 |

`production`や`gantt_chart`は外部アプリケーションの用途であり、IoTKit coreのkindにはしません。Adapterは入力された`observation_id`、`series_id`、sequence、時刻、値を作り直しません。

Adapterは設定のschema versionとcapabilityを検証し、exactly one topic、payload、QoS、retainを返すか、型付きerrorを返します。同じ入力と設定は同じ結果を返します。credential、Broker接続、retry、durable outbox、semantic evaluationはIoTKit Edgeが所有します。

`iotkit.mqtt-json.v1`は全汎用kindを意味変更せず出力する共通Adapterです。`yokakit.mqtt.v1`はYokaKitのpurpose-bound contractへ変換するAdapterで、対応しない`numeric`などを用途へ推測変換しません。別の外部サービスは同じ境界へ新しいAdapterとして追加します。
