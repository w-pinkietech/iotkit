# 純粋リファクタリング評価器

日本語 | [English](README.md)

状態: **実験的／レポート専用**（[#212](https://github.com/w-pinkietech/iotkit/issues/212)）。
[title-free v1 の記録レポート](reports/issue-212-v1-titlefree.md) の推奨は rollout ではなく
**iterate** である。

## Intent

小さな合成的な構造リファクタリングと、純粋なリファクタリングだと確立できない
変更を、独立に取得した evaluator 応答が区別できるかを測る。versioned corpus は
IoTKit の security、custody、recovery、deployment、public wire/API、現在の product
documentation authority、operator-visible 境界では
意図的に保守的にする。

各 case ID は model に見せる diff の SHA-256 の先頭 12 桁を大文字 16 進数にした
`RF-` 接頭辞付きの値である。case は expected outcome ではなくこの不透明 ID で整列し、
両方の corpus half に両 label が含まれることを regression guard とする。
corpus の title は人間用 answer-key context として残すが、model-visible bundle には
含めない。各 prompt case は `case_id` と `diff` だけを持つ。

## When

この perspective は evaluator 自身を評価するときだけに使う。rubric、corpus、
prompt packaging、score 規則を変えた後、および将来の自動化を提案する前に使う。
通常の PR gate ではない。

## How

1. checked-in の v1 入力を検証する:

   ```bash
   node scripts/pure-refactoring-evaluator.mjs check
   ```

2. 決定的な blinded bundle を一つ出力する:

   ```bash
   node scripts/pure-refactoring-evaluator.mjs prompt > /tmp/iotkit-pure-refactoring-v1.json
   ```

3. 同じ bundle を、同一の pinned configuration を使う少なくとも三つの独立に記録する
   evaluator に渡す。各 evaluator は `single_run_keys` と `case_keys` に合う **一つだけ**
   の run object を返す。recorder は一意な `run_id` と、全 run で同一の空でない
   `model_id`（例: `gpt-5.6-sol/high`）を与える。evaluator ではなく recorder がそれらの
   run object を result container にまとめる。result の例を創作せず、実際に capture した
   run だけを `evaluations/` に記録する。
4. checked-in の title-free v1 capture を score する:

   ```bash
   node scripts/pure-refactoring-evaluator.mjs score --results review/pure-refactoring/evaluations/issue-212-v1-titlefree-gpt-5.6-sol-high.json
   ```

評価器は unknown key / unknown reason code、incomplete / ambiguous run、
version/hash のずれ、case coverage の不一致を拒否する。false-safe、false-reject、
dangerous/adversarial false-safe、repeat-run metric をレポートとしてだけ出す。repeat-run
metric は意図的に分ける:

- **classification agreement** は `proven` / `not_proven` の label だけを比較する;
- **reason-code-set agreement** は code の順序を無視して、controlled code set が完全に
  同じかを比較する;
- **expected-reason misses** は、case が期待する code を一つも含まない decision を数える。

各 error metric は observed な `decisions`、対応する `eligible_decisions`、`rate` を
別々に出す。分母は、全記録 run にわたる該当 expected-case population である。

## Not

- 振る舞い等価性、security、custody、compatibility、release readiness の証明ではない。
- 承認、必須 status、auto-merge trigger、人間の review の代替ではない。各 head の
  `human approval` 境界は引き続き必須である。
- model client、network call、secret store、live pull-request reader ではない。
- customer data、credential、未秘匿の field evidence を記録してよいという意味ではない。

title-free v1 report の推奨は rollout ではなく iterate である。この metric を権威化する
提案は、より広い evidence が得られた後の別 Full-lane decision とする。
