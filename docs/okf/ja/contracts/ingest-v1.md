---
type: Contract Overview
title: "IoTKit取り込み契約 v1の概要"
description: "デバイスまたはAdapterからEdge Nodeへ冪等に観測を渡す契約の入口です。"
language: ja
translation_key: contracts.ingest-v1
status: stable
revision: 1
---

# IoTKit取り込み契約 v1の概要

この文書は境界を理解するための概要です。field、limit、error semanticsを省略しない正式契約は、同じGit revisionの`docs/ingest-contract.md`です。exported Rust wire types、共有fixture、conformance testと合わせて一つの契約成果物として扱います。

取り込み契約は、送信側が一つの`Envelope`をEdge Node collectorへ渡し、項目ごとの結果を含む`Ack`を受け取る境界です。再送時は同じimmutable Envelope、同じ`envelope_id`、同一payloadを保持します。同じIDで内容を変更してはいけません。

送信側は観測対象、測定キー、channel、値、デバイス時刻などの事実を渡します。認証済みsourceまたはprincipalは受信側がbindingから決定し、payload内の自己申告値を権限として信頼しません。collectorは測定レジストリ、値型、上限、台帳、重複を検証してから保存します。

現行bindingは次の二つです。

* 公式のプロセス内Input Adapterは共有ingest clientを使う。
* 契約ネイティブまたは外部デバイスは、既定offのTLS付き`POST /api/v1/ingest`とデバイスcredentialを使う。

HTTP request、item数、文字列、body、同時処理数には固定上限があります。`/validate`は同じ検証を行いますが書込みません。成功したHTTP応答でもitemごとの結果を必ず確認します。一時的な保存障害は決定的な`rejected`へ変換せず、送信側が同一Envelopeを再送できる失敗として扱います。

これらの成果物の不一致は契約defectであり、いずれかへ自動追従させません。
