# IoTKit Edge / Edge Node 命名再設計

## 目的

IoTKitを業務・工場・拠点より手前にある汎用IoTデータ基盤として表現し、現在の
`IoTKit Site`を`IoTKit Edge`、現在の`IoTKit Edge`を`IoTKit Edge Node`へ改名する。
コード、公開契約、永続化、導入物、Consoleで同じ階層を使い、裸の`Edge`が収集ノードを
指す状態をなくす。

## 正式な階層

```text
Device
  -> IoTKit Edge Node
  -> MQTT Broker
  -> IoTKit Edge
  -> Application / Fleet layer
```

- **IoTKit Edge**はraw受理、保管責任、Edge Nodeごとのcursor、query、semantic mapping、
  activation、外部出力、Consoleを所有する論理的な状態境界である。
- **IoTKit Edge Node**はデバイスから収集し、正規化し、耐久bufferとoutboxを保持して
  IoTKit Edgeへ配送する独立ノードである。
- **MQTT Broker**は独立配置可能なtransport依存であり、IoTKit Edgeそのものではない。
- **Fleet layer**は複数の`edge_id`を跨ぐ任意の上位層であり、cloudや本社サーバー等の
  物理配置を意味しない。

## 統一語彙

| 現行 | 新しい正本 |
|---|---|
| IoTKit Site | IoTKit Edge |
| IoTKit Edge（収集ノード） | IoTKit Edge Node |
| `site_id` | `edge_id` |
| `edge_node_id` | 維持 |
| Site activation | Edge Node activation |
| Site-managed | Edge-connected |
| Site Console | IoTKit Console |
| site-scoped / site-local | Edge-scoped / Edge-local |
| site-level query | Edge-scoped query |
| site-wide output | Edge-wide output |

`Edge`単独は収集ノードの意味では使用しない。親は`IoTKit Edge`、子は技術文書で
`Edge Node`と書く。`Node`単独も使用しない。日本語Consoleでは子を一貫して
`収集ノード`と表示する。

## Identity

- `edge_id`はIoTKit Edgeが所有するデータ、設定、保管責任、activation grant、backup、
  外部出力の既定source namespaceを識別する。host、process、Broker、Edge Nodeを識別しない。
- 新規IDは`edge-` + 32桁lowercase hexとする。
- `edge_node_id`、`ledger_epoch`、`(edge_node_id, ledger_epoch, pub_seq)`は変更しない。
- 外部出力の一般契約である`source_id`は維持し、IoTKit Edge標準出力では`edge_id`を使う。
- MQTT custody topic `iotkit/v1/edge-nodes/{edge_node_id}/...`は変更しない。

## 公開契約と実行物

| 現行 | 新名称 |
|---|---|
| Go `iotkit-site` | `iotkit-edge` |
| Rust `iotkit-edge` | `iotkit-edge-node` |
| `iotkit-edgectl` | `iotkit-edge-nodectl` |
| `/api/v1/edges`（子） | `/api/v1/edge-nodes` |
| `/equipment/edges/...` | `/equipment/edge-nodes/...` |
| `Edge`, `EdgeState`, `edge_ref`（子） | `EdgeNode`, `EdgeNodeState`, `edge_node_ref` |
| `site_meta` | `edge_meta` |
| child `edge_*` table/column | `edge_node_*` |
| `IOTKIT_SITE_*` | `IOTKIT_EDGE_*` |

中央と収集ノードの実行物、service、container、MQTT client ID、credential principalは
必ず別namespaceにする。中央は`iotkit-edge-*`、収集ノードは`iotkit-edge-node-*`を使う。
中央credentialは`archive`、`output`等の役割名も含める。

## Console

- 製品名は`IoTKit Console`。
- 親は必要な箇所だけ`IoTKit Edge`、子は常に`収集ノード`と表示する。
- `Edge ID`という曖昧な表示を禁止し、`IoTKit Edge ID`と`収集ノードID`を分ける。
- 子の登録操作は`収集ノードを登録`とする。
- 親での受信時刻は`IoTKit Edge受信日時`とする。
- 工場・現場を前提とするコピーは、`システム概要`、`画面に表示する名前`等の汎用語へ変える。
- 登録説明は「登録前の値はIoTKit Edgeへ送信されず保存履歴に含まれない」とし、即時の
  物理削除を示唆しない。

## Cutover

公開前の開発段階であり、ユーザーが開発用DBの破棄を承認したため、旧名称のruntime alias、
旧API route、旧JSON field、旧環境変数、旧DB schemaの互換層は残さない。既存の開発用
IoTKit Site DBと実験用Edge Node DBは破棄し、新しい名前のDBを初期化する。

実運用でIoTKit Siteがすでにcustodyを引き受けたDBを将来移行する場合は、この開発用cutoverを
流用しない。未ack/outboxのdrain、暗号化backup、offline migration、credential失効を含む別計画が必要である。

historicalな`docs/superpowers/specs`と`docs/superpowers/plans`は過去の記録として一括改変しない。
正本の`docs/redesign`、現行運用文書、コード、テスト、公開契約のみを新語彙へ統一する。

## 完了条件

1. 現行コード、正本文書、公開契約、Consoleに製品概念としての`IoTKit Site`が残らない。
2. 収集ノードを裸の`Edge`と呼ぶ公開型、route、画面文言が残らない。
3. `edge_id`と`edge_node_id`の検証、永続化、activation、backup、外部出力が分離される。
4. 新しいバイナリ名、service、compose、bootstrapでクリーン導入できる。
5. MQTT custody、activation、accepted-through、Console、backup/restore、外部出力の検証が通る。
