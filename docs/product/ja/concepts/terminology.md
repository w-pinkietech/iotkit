---
type: Concept
title: "IoTKit用語"
description: "IoTKitの構成要素、識別子、配送責任に使う主要な用語を定義します。"
language: ja
translation_key: concepts.terminology
status: stable
revision: 1
---

# IoTKit用語

| 用語 | 定義 |
|---|---|
| Device | 物理状態を測定・検出する末端機器。センサーや送信機を含みます。 |
| Input Adapter | ベンダーまたはプロトコル固有の入力を汎用取り込み契約へ変換する部品です。 |
| IoTKit Edge Node | デバイスの近くで収集、正規化、耐久バッファ、再送を行う実行単位です。 |
| Internal MQTT Broker | Edge NodeとIoTKit Edgeの間でメッセージを運ぶ標準Brokerです。保存責任の権威ではありません。 |
| IoTKit Edge | 複数Edge Nodeのデータを耐久保存し、Console、意味付け、履歴、外部出力を提供する集約サービスです。 |
| Output Adapter | 汎用観測を一つの外部アプリケーション用topicとpayloadへ決定的に変換する部品です。 |
| custody | データを失わず保管する責任です。IoTKit Edgeの耐久commit後にだけEdge Nodeから移ります。 |
| `edge_id` | 一つのIoTKit Edgeの管理範囲を識別するIDです。工場IDではありません。 |
| `edge_node_id` | 一つのEdge Nodeを識別するIDです。 |
| series | 同じ対象・測定を時系列として連続的に扱う単位です。 |
| observation | 時刻、値、型、identityを持つ一回の観測です。 |
| quarantine | 保存はするが、解除されるまで外部配送やルール評価に使わない状態です。 |

「gateway」は製品構成要素の正式名称として使いません。IoTKit Edge Nodeは単なる中継器ではなく耐久バッファを持ち、IoTKit Edgeは集約・意味付け・外部出力を担うためです。
