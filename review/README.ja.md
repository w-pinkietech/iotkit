# IoTKit レビュースイート

日本語 | [English](README.md)

`review/` は、**この repository が変更をどうレビューするか**の正本です。
単一 skill・単一カタログ・製品契約ではなく、複数の **perspective（視点）** を
持つスイートです。

各 perspective は次の 4 点を答えます。

| 要素 | 意味 |
|---|---|
| **Intent** | どのリスク／品質観点に焦点を当てるか |
| **When** | いつ使うか（毎回、path/concern 一致時、Full レーンのみ、…） |
| **How** | チェックリスト、カタログ、selector、短い手順 |
| **Not** | 何を **証明しない**か（特に: 安全証明ではない） |

機械的な path selector（battle-tested の routing など）は任意です。出す結果は
**下限**です。選択 0 件、未対応 path、CI 緑は、変更が安全だという意味では
ありません。

製品の権威は `docs/product/`（OKF はパッケージ形式）のままです。このスイートは
プロセスと品質レビューであり、第二の製品コーパスではありません。

## Perspectives

| Perspective | 状態 | Intent（短） | 入口 |
|---|---|---|---|
| **battle-tested** | 有効 — **第一 perspective** | 再発させてはいけない運用失敗；現場証拠の triage | [battle-tested/README.ja.md](battle-tested/README.ja.md) |

将来の perspective（ここでは未定義）の例: secrets 扱い、issue-scope のずれ、
公開 contract、Console 操作者 journey、layer 規則。Intent が違うものは
battle-tested カタログに押し込めず、`review/` 直下の兄弟 directory に同じ
Intent / When / How / Not 形で足します。

## スイートの使い方

1. このファイルを開き、変更に合う perspective を選ぶ。
2. 製品や運用に触れる差分では **battle-tested** を常に検討し、path や concern が
   当たりそうなら selector を実行する:

   ```bash
   node scripts/battle-tested-review.mjs select --base origin/master
   ```

3. 上表の他の有効 perspective があれば適用する。
4. 選んだ ID（例: `BT-NNN`）または「該当なし」の具体理由を記録する。path では
   見えない semantic concern も残す。
5. レビューした失敗経路に合わせた検証を行う
   （[`.agents/review-and-verification.md`](../.agents/review-and-verification.md)）。

battle-tested perspective 用の agent skill:
[`.agents/skills/iotkit-battle-tested-review/SKILL.md`](../.agents/skills/iotkit-battle-tested-review/SKILL.md)。
skill は **一 perspective の実行手段**であり、レビュースイート全体の定義では
ありません。

## 共通規則

- **選択 0 件 ≠ 安全。** semantic と contract のリスクは reviewer が担う。
- カタログ項目や checklist は **review question** であり、機能追加の許可や
  本番 ready の証明ではない。
- catch-all カタログを育てない。Intent が違うなら新しい perspective を足す
  （例: secrets と運用失敗モード）。
- credential、顧客識別、生の現場成果物は issue / PR / カタログ link に入れる前に
  秘匿化する。
