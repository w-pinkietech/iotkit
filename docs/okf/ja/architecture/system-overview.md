---
type: Architecture
title: "IoTKitシステム全体像"
description: "IoTKitの配置、入力経路、保管責任、意味付け、外部出力の流れを説明します。"
language: ja
translation_key: architecture.system-overview
status: stable
revision: 1
---

# IoTKitシステム全体像

```text
ベンダー固有デバイス -> Input Adapter --+
契約ネイティブデバイス -> HTTPS ingest -+-> IoTKit Edge Node
  -> internal MQTT Broker
  -> IoTKit Edge
  -> 汎用的な意味を持つObservation
  -> Output Adapter
  -> external MQTT Broker
  -> 外部アプリケーション
```

Input Adapterはデバイス通信とベンダー固有形式の翻訳を担当します。契約を直接実装できるデバイスは、認証付きHTTPS ingestを使えます。どちらの経路もEdge Nodeの同じcollectorへ入り、receiverが認証済み送信者を決定します。

Edge NodeはSQLiteへ観測と配送待ち状態を同一transactionで保存します。activation前の観測はローカルに留まり、後から保管責任ストリームへ再送されません。quarantine中の観測には配送状態を作らず、解除時もactivationとpublication admissionを通ったものだけを配送できます。

MQTTのPUBACKはBrokerへのtransport到達だけを示します。IoTKit Edgeがraw recordと連続cursorを選択中の正本DBへcommitし、対応する`accepted-through`を返した時点で保管責任が移り、Edge Nodeは対象データを削除可能にできます。

IoTKit Edgeは保存済みraw dataを変更せず、別の段階で現場設定に基づく汎用意味へ写像します。Output AdapterはそのObservationから外部アプリケーション固有のtopicとpayloadを作ります。外部出力の障害はraw custodyを止めません。

IoTKit Edgeの正本DBは、一つの導入につき`embedded`（SQLite）または`postgres`（PostgreSQL）のどちらか一つです。両者は同じ製品契約を実装し、実測した容量範囲内で利用します。二重書込みや障害時の無断fallbackは行いません。

関連契約: [取り込み](../contracts/ingest-v1.md)、[保管責任](../contracts/edge-node-custody-v1.md)、[Input Adapter](../contracts/input-adapter-v1.md)、[Output Adapter](../contracts/output-adapter-v1.md)。
