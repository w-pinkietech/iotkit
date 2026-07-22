---
type: Concept
title: "IoTKit製品モデル"
description: "IoTKitがセンサー観測の収集、保全、意味付け、外部出力で担う範囲を定義します。"
language: ja
translation_key: concepts.product-model
status: stable
revision: 1
---

# IoTKit製品モデル

IoTKitは、センサーと工場・業務アプリケーションの間に置く再利用可能なIoT基盤です。異なるセンサーを接続し、通信断や停電をまたいで観測を保全し、現場担当者が汎用的な意味を設定し、外部アプリケーション向けの版管理されたメッセージへ変換します。

IoTKit自身は、工場、製品、工程、作業指示、OEE、生産実績、業務アラームを所有しません。これらはYokaKitなどの外部アプリケーションの責務です。BravePIとYokaKitは最初に検証する統合先ですが、IoTKitの中核モデルではありません。

IoTKitの三つの価値は次のとおりです。

1. Edge Nodeが上位系の停止中も収集と耐久保存を継続する。
2. Input Adapter、取り込み、保管責任、Output Adapterの境界を公開・版管理する。
3. 現場担当者がIoTKit Edge Consoleから現在値と履歴を確認・検索・出力し、表示、意味付け、外部出力の設定を管理できる。設定変更は既存のraw dataやsemantic historyを書き換えない。

一つのIoTKit Edgeは複数のEdge Nodeを管理できます。複数の`edge_id`を横断する管理は、IoTKitの上位に置く任意のfleetまたは業務層の責務です。

関連項目: [用語](terminology.md)、[システム全体像](../architecture/system-overview.md)。
