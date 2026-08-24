<!-- Language guidance / 言語の案内:
Keep this template's required headings and controls unchanged. Write generated natural-language headings/body in the requested language. If bilingual output is explicitly requested, use separate `English` and `日本語` sections; this bilingual template does not itself request bilingual output. Keep product names, API/CLI names, identifiers, state values, protocol names, code, and standard technical proper nouns in original spelling.
このテンプレートの必須見出しと操作項目は変更せず、生成する自然言語の見出し・本文だけを依頼された言語で書いてください。両言語が明示的に求められた場合は `English` と `日本語` のセクションを分けてください。この二言語テンプレート自体は、二言語出力を求めるものではありません。製品名、API/CLI名、識別子、状態値、プロトコル名、コード、標準的な技術上の固有名詞は原綴りを保ってください。
-->

## Summary / 概要

<!-- What changed and why? / 何をなぜ変更しましたか。 -->

## Product docs impact / 正本への影響

<!-- Required. Lasting product facts go in docs/product/ (ja+en; bump the shared revision when concept content changes, not for a path-only move).
     Temporary notes stay on the issue/PR. See AGENTS.md "Keep product docs current".
     docs/product is packaged as OKF v0.2; OKF is the format, not a second corpus.
     Lower-bound selector (empty ≠ safe): node scripts/product-docs-impact.mjs select --base origin/master -->

- Impact selector candidates / セレクタ候補:
  <!-- paste notable candidates from the selector, or "none (unmatched/empty — still judged by hand)" -->
- Updated product-doc paths / 更新した正本:
  <!-- e.g. docs/product/{ja,en}/contracts/ingest-v1.md (revision N→N+1) or "none" -->
- No product-docs update reason / 更新しない理由:
  <!-- Required when paths are "none" -->

## Verification / 検証

<!-- Commands and results. / 実行したcommandと結果。 -->

## Battle-tested review

<!-- Run: node scripts/battle-tested-review.mjs select --base origin/master -->

- Related IDs / 関連ID:
- Field report, regression test, or runbook promotion / 現場報告、回帰test、runbookへの反映:

<!-- Use "none" with a reason when no entry applies. Do not paste secrets, customer data, raw logs, configuration, databases, MQTT topics, or payloads. -->
