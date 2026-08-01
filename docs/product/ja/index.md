# IoTKit 日本語ドキュメント

## 基本概念

* [製品モデル](concepts/product-model.md) - IoTKitが担う価値、責務、対象外を定義します。
* [用語](concepts/terminology.md) - Device、Edge Node、IoTKit Edgeなどの語を定義します。

## 構成

* [システム全体像](architecture/system-overview.md) - 配置、データフロー、保存責任を説明します。

## 公開契約

* [取り込み契約 v1](contracts/ingest-v1.md) - デバイスからEdge Nodeへ観測を渡す完全な契約です。
* [Edge Node保管責任契約 v1](contracts/edge-node-custody-v1.md) - Edge NodeからIoTKit Edgeへの完全な耐久配送契約です。
* [Input Adapter契約 v1](contracts/input-adapter-v1.md) - センサー統合をコアから分離する完全な境界です。
* [Output Adapter契約 v1](contracts/output-adapter-v1.md) - 外部application向けの完全な変換契約です。

## 運用

* [試用profile](operations/trial-profile.md) - 証明書やBrokerの設計なしでloopback限定のsample journeyを開始します。
* [Edge Node hardware復旧クイックガイド](operations/edge-node-hardware-recovery.md) - Backup有無を判断し、印刷用の現場checklistを使います。
* [導入と復旧](operations/installation-and-recovery.md) - 導入、日常確認、証明書、バックアップ、復旧の手順です。
* [Storage容量](operations/storage-capacity.md) - SQLiteとPostgreSQLの再現可能な容量回帰smokeです。
* [OKF 任意メタ](operations/okf-optional-meta.md) - `sources` / `generated` / `verified` をいつ書くか（任意・非必須）。
