# Battle-testedレビュースイート

日本語 | [English](README.md)

このdirectoryは、IoTKitの変更によって再発させてはいけない現場の失敗を扱う、
repository固有の小さな索引です。製品契約、incident database、機能backlogではなく、
列挙したすべての条件をIoTKitが実地で経験済みだと主張するものでもありません。

レビュー項目の正本は`catalog.json`だけです。Codexと人間のreviewerはcatalog全体を
読み込まず、selectorを使います。

```bash
node scripts/battle-tested-review.mjs select --base origin/master
node scripts/battle-tested-review.mjs select --base origin/master \
  --concern custody
node scripts/battle-tested-review.mjs concerns
```

Path routingは選択の下限です。公開契約、認証、custody、data loss、migration、restore、
外部作用の意味を変える場合、pathで選ばれなくても`--concern`を追加します。未対応pathや
選択0件は、その変更が安全だという意味ではありません。

## 現場報告から持続する証拠へ

1. **先に秘匿化する。** credential、鍵、token、顧客名、工場名、network識別情報、
   serial番号、MQTT topic/payload、DB、設定file、その値を含むscreenshotを除きます。
2. **報告をtriageする。** 重複、影響、再現性、IoTKitの責務かを確認します。報告された
   だけでは確認済みの証拠にしません。Issueを閉じるときは、重複、情報待ち、catalog採用、
   修正・guard済み、変更なしでrisk受容、対象外、security経路へ移動、のいずれかをcommentへ
   記録します。すべての報告からcatalog変更を作りません。
3. **最小の持続する成果を選ぶ。**
   - 有力だが未確認なら`hypothesis`に留める。
   - 未確認の現場報告は`field-reported`とする。
   - Maintainerが発生を確認した場合は`field-observed`とする。
   - 制御された再現がある場合は`reproduced`とする。
   - 再現可能な製品動作にはfocused regression testを追加する。
   - 運用で対処する障害にはrunbookを紐付ける。
4. **将来のreviewを改善する場合だけ索引へ入れる。** 発生元Issueまたは既存のrepository内
   証拠をlinkし、test手順やrunbook本文を項目へ複製しません。
5. **役に立たなくなった項目は削除・統合する。** 安定IDは追跡用であり、永久保存義務では
   ありません。

Catalog項目はreview questionであり、製品機能を追加する許可ではありません。仮説を実装課題へ
変える前に、再現または重大な損失へ至る信頼できる経路が必要です。

## Catalog規則

- 1項目で1つの失敗だけを扱い、質問を短く保つ。
- catch-all path prefixを使わない。
- `provenance`と`guards`は既存fileまたはGitHub Issueへlinkする。
- Capacity testをdisk-full testと同一視しない。
- Sensor device交換とEdge Node computer交換を混同しない。
- 通常PRのCIではcatalog構造とroutingだけを検査する。重い障害testとrelease gateは
  focused testまたはrelease時の検証に残す。

次で検証します。

```bash
node scripts/battle-tested-review.mjs check
node --test scripts/tests/battle-tested-review.test.mjs
```
