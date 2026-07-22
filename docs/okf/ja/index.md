# IoTKit 日本語ドキュメント

## 基本概念

* [製品モデル](concepts/product-model.md) - IoTKitが担う価値、責務、対象外を定義します。
* [用語](concepts/terminology.md) - Device、Edge Node、IoTKit Edgeなどの語を定義します。

## 構成

* [システム全体像](architecture/system-overview.md) - 配置、データフロー、保存責任を説明します。

## 公開契約

* [取り込み契約 v1の概要](contracts/ingest-v1.md) - デバイスからEdge Nodeへ観測を渡す契約の入口です。
* [Edge Node保管責任契約 v1の概要](contracts/edge-node-custody-v1.md) - Edge NodeからIoTKit Edgeへの耐久配送契約の入口です。
* [Input Adapter契約 v1の概要](contracts/input-adapter-v1.md) - センサー統合をコアから分離する境界の入口です。
* [Output Adapter契約 v1の概要](contracts/output-adapter-v1.md) - 汎用観測を外部アプリ向けに変換する境界の入口です。

## 運用

* [導入と復旧の概要](operations/installation-and-recovery.md) - 導入、日常確認、証明書、バックアップ、復旧の入口です。
