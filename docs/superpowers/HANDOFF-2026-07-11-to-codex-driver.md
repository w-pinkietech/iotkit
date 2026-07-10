# 引き継ぎ: メイン駆動を Claude Code → codex CLI へ (2026-07-11)

このファイルは、これまで Claude Code のメインエージェントが保持していた「会話コンテキスト上の
進捗」を、codex CLI がメイン駆動として引き継ぐための一枚ものスナップショットである。設計正本でも
台帳でもない——**移行時点の生の作業状態**を失わないための橋渡し。恒久的な正本は末尾の「正典の所在」
を参照。

移行の理由: Claude 側のトークンクォータが律速で、codex 側の余力を使い切れない。以後の主駆動を
codex CLI に移す。

## 0. 役割の逆転(AGENTS.md との差分——重要)

`AGENTS.md` は「codex=呼ばれる側の per-task 実装ワーカー」向けに書かれている(「git commit しない」
「指定タスクだけ」)。**メイン駆動になった codex には、そのうち以下が変わる**:

- コミットは自分で行う(呼び出し側がいない)。コミットスタイルは CLAUDE.md の Commit Style。
  トレーラは `scripts/trailer.sh`(引数はモデル名の検出用——codex 駆動なら実際の生成者に合わせる)。
- spec/plan/レビュー dispatch/ユーザー対話も自分の責務になる(旧: Claude メインの領分)。
- **クロスベンダーレビューは維持する(2026-07-11 ユーザー決定)**。codex がメインで書くので、
  Claude 側を codex から呼ぶ: `scripts/claude-review.sh <prompt-file> <label>`(**静的** read-only
  =plan+disallow+no-settings。出力は同じ `/tmp/codex-runs/`、effort 既定 max、強モデルは
  `CLAUDE_REVIEW_MODEL` でピン)。手順は codex.sh review と claude-review.sh へ同一プロンプトを
  並走 dispatch。**非対称**: codex 側はコマンド実行可(cargo test 等)、Claude 側は静的(Bash 不可)
  ——実行依存のバグは codex が主担当。AGENTS.md「クロスベンダーレビュー(メイン駆動時)」節に正文。
  **05db5da で正文化した「待たない運用+消費ベース tier+確認ラウンドの規律」は、レビュアーが
  誰であっても成り立つ規律なので維持する**(CLAUDE.md Workflow Rules)。

## 1. いまどこにいるか (git 実状態 2026-07-11)

- iotkit-next @ `05db5da`(origin/master 同期, クリーン)。iot @ `05db5da`... ではなく
  iot リポジトリ(iotkit-redesign)は別 HEAD——移行前に両方 `git log -1` で再確認せよ
  (記憶でなく git が正典)。
- 直近の完了: 構造監査ラン → Grok 第三ベンダーレビューのトリアージ記帳(6b74c5f)→
  review 速度方針の正文化(05db5da)。
- **現在地: 計画6(R2 ネットワーク入口 + R19 入口側認証)の brainstorming 中**。
  パイプライン: brainstorming → codex-eval-spec → writing-plans → codex-eval-plan →
  codex-impl-loop → PR。まだ spec は書いていない(設計セクションのユーザー承認前)。

## 2. 計画6: 確定済みスコープ (再議論しない)

- 対象: R2 HTTP 入口 + R19 入口側認証。設計正本 = D11(+D1/D5 既決)。
- OUT: MQTT ingest(別計画、先頭タスク=D11保留の絞りワイヤ表現)、ペアリング窓経路(計画9、D13)、
  R22 snapshot 秘密投入+暗号化コンテナ(**計画6.5 に分割——2026-07-10 ユーザー裁定**。
  計画5 spec §9 の「計画6で解消」を「6.5で解消」に読み替える。**この読み替えはまだ台帳・spec に
  未記帳**——計画6 spec を書くとき、または次のコミットで反映すること)。
- 無認証面ゼロ(帯外経路のみ)。具体値(流量クラス段階・絞りパラメータ・ステージング上限・
  鮮度ウィンドウ)は設定化した暫定値、実測確定は命名済み別計画。

## 3. 計画6: 設計ドラフト (ユーザー承認前。brainstorming の叩き台)

「計画6への持ち込み」8項目(台帳 = docs/superpowers/plans/wave1-plan5-deferred-hardening.md、
6b74c5f で確定)を Global Constraints へ写像する。設計セクション案:

### A. 構成と置き場
- 新 crate `iotkit-ingest-http`(axum。`gateway/src/api`=制御面とは別物。認証・レート制限境界が
  違う、D1:142)。check-layers に **INGRESS 分類**新設(ingest-contract+ingest-client+core側最小
  依存のみ、engine/supervision 依存禁止)。architecture.md の地図・置き場表・層規則を同時更新。
- リスナー既定オフ。有効化・bind変更・平文opt-in = 工事層 R14 op(型付き・監査付き、D11決定7)。
  bind は site_local_cidr+許可IF で検証(平文=プライベートアドレス限定 MUST)。
- gateway composition root が有効時のみ spawn。公式アダプタのみの現場は攻撃面ゼロのまま。

### B. デバイストークンと IngestPrincipal
- R14 ops: device token issue/revoke/reissue。発行は device add と一体(D2/D5)。ハッシュ保存
  (新テーブル device_tokens)。失効=人間のみ+理由コード、R12 stale 報告、replace-hardware/廃棄で
  旧トークン失効必須(D11決定3/8)。
- トークン→subject スコープ束縛。`IngestPrincipal { device_id, subject_scope, flow_class, profile }`
  を認証層が構築し IngestRequest に**エンベロープと別載せ**。envelope.source=診断メタデータ、
  principal 不一致=reject+侵害シグナル監査(D11決定2、D1:198-201)。
- profile は `simple_bearer` のみ実装。`signed_seq`/`provisioned_key` は名前予約のみ(D11決定3)。
- **1:1 トークン(subject_scope が単一 subject)は subject_hint 省略可 → principal から解決**
  (D5決定1)。多 subject トークンの省略 = 終端 UnknownSubject(現行どおり)。これが持ち込み1の
  「D5 1:1 省略解決」の実装。

### C. 流入制御 (申告制、D11決定4)
- flow_class: 登録時申告・既定クラスあり・変更は人間のみ(R14)。帰属単位=認証送信者。
- 執行: token bucket(認証送信者単位)+グローバル上限。超過=HTTP 429+Retry-After(ボディ処理前)。
  **終端 rejected には決して写像しない**(可逆な絞りを spool 送信者がデータ破壊する事故を防ぐ、
  D11決定4)。解除ヒステリシス+エピソード集約アラーム(audit+R12水位)。
- 検算: device add・クラス変更の R14 事前条件で申告合計 vs 実測体力(暫定値=設定)。超過=明示承認
  +capacity_debt 記録。水位は R12 常時公開。
- spoolなし送信者への正直さ: `throttled_drop_count` を R12/監査に。契約文書に明記。
- token-bucket 採用は D11 保留(絞りアルゴリズム)の解決として設計正本へ**還流記録**すること。

### D. 受理面 (D1既決の実装+D11決定5)
- 認証はボディ読み込み前(未認証=確保ゼロ 401)。ボディ上限・タイムアウト・同時接続上限・
  有界キュー(満杯503)・dedup TTL/上限。認証失敗の監査+失敗レート減速。
- staged sightings 有界化: 送信者ごと+全体の2段上限、最古破棄+監査、承認待ち/replace候補/
  画面表示中はピン留め保護、上限到達アラーム(D11決定5)。
- **鮮度ウィンドウ**: 窓超の event_time は拒否(D1:60-65。持ち込み5)。判定はコレクタ側。
- dedup 縮退の監査。

### E. ack 契約の完成 + 契約文書 (持ち込み2/3)
- 拒否詳細に field_path(JSON pointer)+期待スキーマヒント。`ReasonCode::Internal` 削除
  (未使用・D1準拠の生成者なし=T4監査で文書化済み。D1:90/93)。ワイヤ適合テスト同時。
  ingest-contract crate は破壊変更 → バージョン節を文書に。
- `docs/ingest-contract.md` 新設(exit-contract.md の対)。受け入れ基準: 実機で curl 3行が通り、
  文書の写経だけで ESP32 相当の送信者が書ける(architecture.md ペルソナ品質バー)。

### F. 配布時セキュリティ既定 (持ち込み8) —— メニュー検証完了・**ユーザー裁定待ち**
第4節に詳細。ここが計画6 brainstorming の唯一の未決の主要分岐。

### G. 記帳
- D11保留の解決を還流(token-bucket・暫定値の設定化・配布既定)。台帳: 持ち込み節の完了記録、
  R22→6.5、新規繰り延べ(実測確定・MQTT・強度具体値)。

## 4. 未決: 配布時セキュリティ既定 (両ベンダー検証済みメニュー、ユーザー裁定待ち)

**問題**(全て 2026-07-10 コード実物照合済み): 制御プレーン API が既定 `enabled=true`+bind
`0.0.0.0:8443`(config.rs:326-327)。setup モード(admin パスフレーズ未設定)中、`POST
/api/v1/setup/passphrase` が**認証外**(routes.rs:62)——**同一 LAN の誰でも先にパスフレーズを
設定すれば箱を掌握できる**。claim の戦利品は工事層フル(Tier::Construction・30日TTL セッション)。
さらに `reset_passphrase` は既存セッションを失効させない(auth.rs:126-147)ので、ローカルで
パスフレーズを取り戻しても乗っ取りセッションは最長30日生きる。箱は mDNS で自分を広告する(D2)ので
時間レースは自動 claimer に負ける。

**両ベンダー(codex+Fable)が独立に収束した結論**: 守るべき不変条件は「bind 既定」ではなく
**「未認証のネットワーク端末が、ネットワーク越しに箱の掌握(box-claim)を一度も実行できない」**。
claim には所持証明(フラッシュ時投入・箱上で生まれたトークン・ローカルCLI)を必須とする。
bind/site_local_cidr は claim 保証を入れた後の副次的な露出ノブに格下げ。

**ユーザーに出すメニュー(推奨=案1)**:

1. **(推奨) 所持証明付き claim**: SD を焼くときにパスフレーズ/コードを1項目事前投入(boot
   パーティション、初回起動で消費・削除)すれば setup 窓はネットに一度も開かない。未投入なら箱が
   個体別ワンタイム setup code を生成(boot パーティション書き出し+`gatewayctl setup-token` 1回表示、
   journal には出さない)し、UI が1回だけ要求 → 短命 TTL の専有 setup セッションと交換。D13 の
   ブラウザ導線・setup 閉集合はそのまま。D10 登録券/D11 登録コードと同型(単回使用・短TTL・最低
   エントロピー・送信元スロットリング)で新規設計面は最小。**UX コスト**: 焼くとき1項目、または
   忘れたら SD 再マウント/端末接続でコードを1回拾う。
2. **ネット claim 全面禁止**: パスフレーズは事前投入か gatewayctl(ローカル)のみ。最強・実装最小
   (新トークン基盤不要)だが、事前投入を忘れた設置者は必ず SSH/SD 再マウントに落ち、D2/D13 の
   「UI だけでコミッショニング完了」合格基準との整合裁定が別途要る。
3. **受容+可視化(配布延期の暫定)**: 現状受容+自動クローズ+claim 元IP/時刻の騒がしい表示+
   D-14 手順書。摩擦ゼロだが mDNS 広告下で自動 claimer に構造的に負ける。採るなら Grok C1 と台帳
   裁定の明示上書き+受容リスク記録が条件。

**付帯(どの案でも必須)**: `reset_passphrase` のセッション失効連動(auth.rs:126-147 の穴)、claim
監査の UI 表示、D-14 チェックリストとの対、採決を **D13:198 の「setup 窓の追加ガード」予約への
追補**として設計正本に記録(D13 の再審理ではない)。共有イメージへの共通トークン焼き込み禁止・
個体別秘密は DB にハッシュのみ・パスフレーズ設定時に失効・秘密自体は監査しない。

**注意**: 案1/案2 の「Home Assistant 型」という比喩は codex が正した——HA は今も初回管理者作成が
`requires_auth=False`。転用元として不正確なので比喩を使うなら Matter(発見+帯外 passcode)型が近い。

## 5. 直近で決めたレビュー速度方針 (05db5da、維持する)

CLAUDE.md Workflow Rules の3バレットが正文(条件はそこを読む。ここに再記述しない=drift 防止)。
骨子だけ: レビュー並走+消費ゲート(未決成果物は飛行中 読み書き凍結、下流消費は SETTLED まで禁止、
未決は scratchpad `review-pending.md` に**内容ハッシュ付き**でディスク記録、fail-closed)/
ベンダーのゼロ判定はレビューしたハッシュに紐づく/確認宛先は fix・棄却した C/I のオーナー(棄却=
Main 発の不在主張=確認必須)/tier は「下流が根拠として読むか」一問、迷ったら両ベンダー。

**注意: scratchpad は tmpfs・セッション固有**。この引き継ぎ後は消える。移行後 codex が同じ規律を
使うなら、review-pending の置き場をリポジトリ内の固定パスにするか、codex 側の永続領域に移すこと
(fail-closed 節: ファイル不在は SETTLED の証拠ではない)。

## 6. 正典の所在 (恒久。この引き継ぎ文書より優先)

- **設計正本**: `../docs/redesign/`(D1〜D13、責務台帳 R1〜R23、terminology、adr-inventory)。
  特に計画6は **D11-ingress-authentication.md** が本体。
- **構造正本**(crate地図・置き場・層規則・ペルソナ): `docs/architecture.md`。機械検査=`scripts/check-layers`。
- **ワークフロー規律**: `CLAUDE.md`(Claude 向け)/ `AGENTS.md`(codex 向け——役割逆転分は本書 §0)。
- **繰り延べ台帳**(計画6が全掃引する): `docs/superpowers/plans/wave1-plan5-deferred-hardening.md`
  ——「計画6への持ち込み」8項目+D-9〜D-18。
- **メモリ**(Claude 側、参考): `../.claude/memory/`(iot リポジトリに symlink)。
  review-speed-policy / iotkit-wave1-status など。
- ハーネス: `scripts/`(codex.sh・claude-review.sh・verify.sh・check-layers・trailer.sh・watchpoints.sh)。
- レビューガイド: `docs/eval/{spec,plan,impl-spec,impl-quality}-review.md`。

## 7. 移行後の最初の一手 (推奨)

1. 両リポジトリの `git log -1` で HEAD を確認(本書の主張を鵜呑みにしない=「語りを信じるな、
   実物を読め」)。
2. §4 のメニューをユーザーに提示し、配布既定を裁定してもらう(計画6 spec の最後の未決分岐)。
3. §3 の設計セクションをユーザーに順に承認してもらう(brainstorming の残り)。
4. 承認後、計画6 spec を `docs/superpowers/specs/YYYY-MM-DD-wave1-plan6-*.md` に書き、
   R22→6.5 読み替えを台帳に反映、codex-eval-spec へ。
