# Lua Transform Layer — 構想メモ

Date: 2026-03-26
Status: Draft

## 背景

`iotkit-next` では `driver -> adapter -> core` の境界を整理し、
ハードウェア依存やプロトコル依存を adapter 側で吸収する方向で進めている。

一方で、現場ごとの運用差分は builtin の固定ロジックだけでは吸収しきれない。
特に以下は現場差分が大きい。

- パトランプの意味付け
- サイクルタイム算出ルール
- 設備ごとのしきい値や窓判定
- MQTT や外部系へ送りたい形への整形

レガシーでは Node-RED がこの役割を広く担っていたが、
自由度が高すぎて構成の見通しと保守性を失いやすかった。

このメモの目的は、Node-RED の再発明ではなく、
`core` の後段に「制約付き transform layer」を置く構想を整理することにある。

## 目的

- preset だけで大半の現場要件を満たす
- preset で足りない差分だけを script で吸収する
- script を使っても `core` と adapter の境界を汚さない
- AI が script を生成しても壊れにくい運用境界を作る
- 現場で dry-run、切り戻し、障害切り分けがしやすい形にする

## 推奨方針

Lua は導入する。ただし「何でもできる script engine」としてではなく、
`core` の正規化イベントの後段に置く **制約付き transform layer** として扱う。

推奨するデータフロー:

`driver -> adapter -> core 正規化イベント -> transform service (Lua) -> DB / MQTT / 通知 / API`

この方針では:

- hardware access
- pairing / scan / DFU
- transport retry
- DB write
- MQTT publish
- file/network access

を Lua から直接触らせない。

## なぜ Lua か

- 軽量で組み込みやすい
- sandbox を作りやすい
- 小さな変換ロジックに向いている
- AI に補助させる前提でも、構文と実行モデルが比較的単純

ただし、採用の本質は Lua 自体ではない。
重要なのは「自由なロジックを末端の transform に閉じ込めること」である。

## 代替案の比較

### 1. preset only

最も安全だが、現場差分に弱い。
パトランプやサイクルタイムのような設備依存ルールを吸収しきれない。

### 2. preset first + constrained Lua transform

推奨案。
8割は preset、残り2割だけ script で逃がす。
堅さと柔軟性のバランスが最も良い。

### 3. full script flow engine

非推奨。
Node-RED の別実装になりやすく、
「フローが本体」になって設計境界が再び崩れる。

## 配置と依存方向

Lua layer は `core` ではなく service 層に置く。

理由:

- `core` は canonical な型とルールに集中させたい
- script engine の依存を `core` に持ち込みたくない
- script 実行、sandbox、state、運用機能は application/service concern である

依存方向のイメージ:

- `core/types` は transform engine を知らない
- transform engine は `core` の canonical input に依存する
- 外部送信や保存は transform engine の出力を Rust 側 service が受けて実行する

## 入力契約

Lua に `AdapterEvent` をそのまま渡さず、
script 専用の versioned DTO を渡す。

例:

```text
TransformInput v1
- timestamp
- adapter_id
- device_key
- sensor_type
- values: { label -> scalar }
- meta:
  - manufacturer
  - part_number
  - connection.kind
  - tags
```

ここでの重要点:

- `Vec<f64>` の生配列ではなく label 付き map にして渡す
- adapter 固有の raw 値や protocol 番号はここに出さない
- version を持たせて将来の互換性を保つ

AI に script を書かせる場合も、この形の方が圧倒的に安定する。

## 出力契約

Lua の出力も自由 JSON ではなく、制約付き API にする。

想定する最小 API:

- `emit_point(name, value, tags)`
- `emit_state(name, value)`
- `emit_event(name, level, message, fields)`
- `drop()`

ポイント:

- script は「何を出したいか」だけ決める
- 実際の publish / save / notify は Rust 側が担当する
- script から外部 I/O を直接呼ばせない

## preset 優先モデル

運用の第一選択は custom script ではなく preset とする。

想定:

- `passthrough`
- `scale_offset`
- `threshold_window`
- `gpio_debounce`
- `edge_cycle_time`
- `patlamp_basic`

推奨順序:

1. preset を選ぶ
2. preset に params を入れる
3. それでも足りないときだけ custom Lua を使う

この順序にすると、現場の多くはコードを書かずに済む。

## custom Lua の役割

custom Lua は escape hatch であり、本体ではない。

向いている用途:

- 工場固有のパトランプ解釈
- サイクル開始/終了の判定差分
- 複数値からの設備状態推定
- 出力 payload の現場フォーマット調整

向いていない用途:

- hardware 制御
- adapter lifecycle 制御
- DB schema 依存の処理
- 外部 API 直接呼び出し
- 長い業務フロー

## 状態管理

サイクルタイムや edge 判定のため、完全 stateless では足りない。
ただし重い workflow engine にしないため、
state は **小さく、局所的に、失ってもよいもの** に限定する。

方針:

- per-device / per-script の小さな state を持てる
- in-memory を基本とする
- TTL を持つ
- restart で失われてもよい設計を優先する
- 永続状態が必要なロジックは script に寄せない

## sandbox と安全性

最低限必要な制約:

- timeout 制限
- memory 制限
- `io`, `os`, `package`, `debug` の無効化
- file/network access 禁止
- script failure で取り込み全体を止めない
- 失敗時は adapter/core ではなく transform failure として隔離する

script は「壊れても周囲を巻き込まない」ことが最優先。

## 運用機能

実際に現場で使えるかは、script 実行機能そのものより周辺運用で決まる。

最低限ほしい機能:

- sample input を使った dry-run
- fixture ベースの簡易テスト
- shadow mode
- 現在適用中の preset / script 名の表示
- last success / last error の表示
- version 履歴
- rollback

特に AI が script を書く前提なら、
「そのまま有効化」ではなく「fixture を通してから有効化」の流れが必要。

## 推奨 API イメージ

Lua には素の自由記述より、helper を厚く提供する。

例:

```lua
function transform(ctx, input, state)
  if rising(input.values.green_lamp) then
    state.started_at = input.timestamp
  end

  if falling(input.values.green_lamp) and state.started_at then
    emit_point("cycle_time_ms", input.timestamp - state.started_at, ctx.tags)
    state.started_at = nil
  end
end
```

ここで使う `rising`、`falling`、`emit_point` などは
組み込み helper として提供する想定。

## 非目標

この構想では以下は扱わない。

- Node-RED 相当の full flow editor
- script からの任意 I/O
- script からの device control
- script を通した adapter/core 境界の変更
- 現時点での永続 state machine

## 最初の着地点

初手は大きくしすぎない方がよい。
最小構成としては以下を勧める。

1. `TransformInput v1` を固定する
2. `emit_point` と `drop` だけ先に実装する
3. builtin は `passthrough`, `threshold_window`, `edge_cycle_time`, `patlamp_basic` から始める
4. custom Lua は opt-in にする
5. dry-run と last error 表示を先に作る

## オープンクエスチョン

- transform の適用単位は device か、sensor_type か、machine class か
- state の TTL とリセット契機をどう定義するか
- output を時系列向け point と event に二分するか
- UI から preset を編集できる範囲をどこまで許すか
- script version 管理を DB と file のどちらで持つか

## 結論

現場差分を吸収する余白として script layer を持つ判断は妥当。
ただし成功条件は「Lua を入れること」ではなく、
`preset 優先`、`transform に責務を限定`、`sandbox と運用機能を先に設計` の3点を守ることにある。

この境界を守れば、レガシーより堅くしながら、
現場にはむしろ使いやすい仕組みに寄せられる。
