# 純粋リファクタリング評価器

日本語 | [English](README.md)

状態: **実験的／レポート専用**（[#212](https://github.com/w-pinkietech/iotkit/issues/212)、
[#214](https://github.com/w-pinkietech/iotkit/issues/214)）。
[title-free v1 の記録レポート](reports/issue-212-v1-titlefree.md) の推奨は rollout ではなく
**iterate** である。[historical v2 report](reports/issue-214-v2-historical.md) も human decision
を rollout や stop ではなく **iterate** と記録する。

## Intent

小さな構造リファクタリングと、純粋なリファクタリングだと確立できない変更を、独立に
取得した evaluator 応答が区別できるかを測る。versioned corpus は IoTKit の security、
custody、recovery、deployment、public wire/API、現在の product documentation authority、
operator-visible 境界では意図的に保守的にする。V1 は合成 corpus、v2 は sanitizer と
provenance binding を持つ historical corpus である。

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

2. freeze 済み historical v2 bundle を検証して出力する:

   ```bash
   node scripts/pure-refactoring-evaluator.mjs check --corpus-version 2
   node scripts/pure-refactoring-evaluator.mjs prompt --corpus-version 2 > /tmp/iotkit-pure-refactoring-v2.json
   ```

   V2 は raw selection-policy bytes を recorder-only provenance、corpus、prompt bundle へ
   結び付ける。Git `index` line の除去は reversible であり、provenance はその正確な text と
   one-based raw position を記録する。validation は source diff を復元して hash を検証してから
   model-visible diff を照合する。PR、commit、title、label metadata は model-visible input
   から除外する。記録済みの capture と report も report-only のままである。

3. 決定的な blinded v1 bundle を一つ出力する:

   ```bash
   node scripts/pure-refactoring-evaluator.mjs prompt > /tmp/iotkit-pure-refactoring-v1.json
   ```

4. 同じ bundle を、同一の pinned configuration を使う少なくとも三つの独立に記録する
   evaluator に渡す。各 evaluator は `single_run_keys` と `case_keys` に合う **一つだけ**
   の run object を返す。recorder は一意な `run_id` と、全 run で同一の空でない
   `model_id`（例: `gpt-5.6-sol/high`）を与える。evaluator ではなく recorder がそれらの
   run object を result container にまとめる。result の例を創作せず、実際に capture した
   run だけを `evaluations/` に記録する。
5. checked-in の title-free v1 capture を score する:

   ```bash
   node scripts/pure-refactoring-evaluator.mjs score --results review/pure-refactoring/evaluations/issue-212-v1-titlefree-gpt-5.6-sol-high.json
   ```

記録済みの v1/v2 comparison は descriptive かつ unpaired である:

```bash
node scripts/pure-refactoring-evaluator.mjs compare \
  --baseline-results V1_RESULTS.json \
  --historical-results V2_RESULTS.json
```

これは count、rate、delta、agreement summary だけを比較する。threshold や rollout authority は
持たず、final report の human decision は **iterate** である。

評価器は unknown key / unknown reason code、incomplete / ambiguous run、version/hash のずれ、
policy/provenance/corpus の drift、case coverage の不一致を拒否する。false-safe、false-reject、
dangerous/adversarial false-safe、repeat-run metric をレポートとしてだけ出す。repeat-run metric は
意図的に分ける:

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
- 記録済みの v2 result や report を rollout authority として扱う理由ではない。

title-free v1 report の推奨は rollout ではなく iterate である。この metric を権威化する
提案は、より広い evidence が得られた後の別 Full-lane decision とする。
