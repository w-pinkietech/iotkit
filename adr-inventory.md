# ADR棚卸し(monojoh-authority docs/adr 0001〜0042)

Status: 確定 (2026-07-02。Sonnetエージェント6体による分担精査+メイン対話での裁定+ユーザー承認済み)
対象: /home/kenta/dev/iot/monojoh-authority/docs/adr/ の42本
用語は [terminology.md](terminology.md) に従う。

## 権威規則(本棚卸しで確定)

> 再設計期間中は `docs/redesign/` 配下(用語集・責務台帳・決定文書D1〜Dn)が**現行の設計権威**であり、
> 矛盾するADR記述を上書きする。ADR本文への反映(supersede/改訂)は本文書の処置案に従って実施し、
> 反映完了までは本文書が新旧の対応表として権威の橋渡しをする。

この規則はADR 0031の新規発見(下記)への暫定回答でもある。決定文書→正式ADRへの昇格ルールは
0031改訂時に恒久化する。

## 集計

| 判定 | 本数 | 対象 |
|---|---|---|
| 維持 | 16 | 0001, 0002, 0004, 0007, 0010, 0014, 0018, 0019, 0020, 0022, 0027, 0034, 0036, 0037, 0040, 0041 |
| 要改訂 | 21 | 0003, 0005, 0008, 0009, 0011, 0012, 0013, 0015, 0016, 0017, 0021, 0023, 0024, 0025, 0026, 0030, 0031, 0033, 0038, 0039, 0042 |
| 廃止(supersede) | 1 | 0006 |
| 統合済み(キュー3=D7 2026-07-03) | 4 | 0028, 0029, 0032, 0035(0030の出口側処置もD7で回収) |

横断事項: 「edge」単独語(用語集の禁止語)が 0005/0023/0025/0026/0027/0028/0029/0032 等に残存。
各ADRの改訂時に「ゲートウェイ[2]」等の配置4段語彙へ機械的に置換する。

## 維持(16本)

| ADR | 一言根拠 |
|---|---|
| 0001 provider-neutral adapter policy | D4の3部品分解が本ADRの原則の精密化。完全整合 |
| 0002 current compatibility target | WSL開発レーン+RPi4B検証はWave 0と整合 |
| 0004 agent-first tooling | R14(型付き操作カタログ・AI/人間共用)の先取り。柱3と一致 |
| 0007 first compatibility scope | 「BravePI経路のみ・BraveJIG対象外」はD3 Wave 0定義と文言レベルで一致 |
| 0010 thin bus and bounded configuration | R9「自由グラフ不採用・型付き設定のみ」と一致。内部busとD1バインディングは別レイヤ(改訂時に脚注推奨) |
| 0014 adapter extensibility without early plugin system | D3 Wave分割・D2§4「配置だけの違い」・D4の4形態と整合 |
| 0018 gatewayctl primary entrypoint | host-agent直接アクセスを例外扱いとする構図はhost-agent縮小後も成立 |
| 0019 bounded job resource | 用語集の有界ジョブ定義そのもの。D5親交換一括replaceでも利用 |
| 0020 canonical control surface envelope | R14要求はresult/details内で表現可能。D1バッチ部分ackとも両立 |
| 0022 latency and resource bounds | D1ヒステリシス2閾値・R17劣化契約と整合。コマンド完了側はキュー5確定後に精緻化 |
| 0027 factory optimization north star | 柱3・AIオペレーター配置([3]/[4])と強く整合。edge語の置換のみ任意実施 |
| 0034 timeseries backend extraction triggers | SQLite単一ストア→トリガー条件で抽出はD1/D5物理表現と整合 |
| 0036 contract-first multi-agent development flow | 開発運用規律。技術決定と別軸で矛盾なし |
| 0037 stable error codes and paginated lists | D1 reason_code・D5カーソル設計と哲学レベルで整合 |
| 0040 github issues work orchestration | 開発プロセス。矛盾なし |
| 0041 two-stage validation feedback loop | 開発プロセス。same/split-deviceプロファイル名の追随は任意 |

## 廃止(1本)

### ADR 0006 host and edge responsibility split
- **supersede先: D4(3部品+4形態)全体、D2§4(アダプタ・コレクタの配置)、terminology.md host-agent項**
- 中心主張「host-agentがローカルデバイスアクセスとアダプタ実行を所有し、edge runtimeはhost-agent経由で会話する」という2階層モデルが骨格ごと消滅。衛星アダプタ/契約ネイティブデバイスはコレクタと取り込み契約(D1)で直接会話し、host-agentを介さない
- 有用な不変条件(衛星側は業務ポリシー・外部API権威・永続化正本・横断オーケストレーションを持たない)はD4アダプタランタイム定義とR8に内包済み。移植作業不要
- 処置: ステータスを`superseded`にし、後継文書3点を明記

## 要改訂(21本)

### グループA: host-agent縮小の波及(0003, 0005, 0008, 0012, 0015)

共通原因: 用語集でhost-agentが「**sudo級特権操作のみ**(再起動・時刻設定・サービス制御等)。
ハードウェアI/O(シリアル/I2C)はアダプタのドライバが直接扱う」に縮小されたこと。

- **0003 host-agent boundary**: 「All hardware access ... live in host-agent」をsudo級限定に書き換え。
  /dev/*・GPIO・I2C・カメラはドライバ直接に分離(システムクロック・サービス制御のみhost-agent専有)。
  「adapters execute in or behind host-agent」を削除し、アダプタ=独立監督単位(R20)+取り込み契約(D1)経由に置換
- **0005 portable host and edge runtime**: 「edge runtime」語を全廃し、衛星アダプタ(D2§4)/
  契約ネイティブデバイス(D4形態④)で書き分け。ESP32等は「北向き=D1、南向き=キュー5確定待ち。
  host-agent仲介なし」に書き換え
- **0008 adapter and collector topology**: 骨子(分散配置可・同一契約)はD2§4/D4に生存。
  「host-agent is the local authority boundary for device access and adapter execution」の主語をアダプタ(3部品)に差し替え
- **0012 gateway-side collector authority**: 核心(コレクタ=常にゲートウェイ側の権威)はD2§4で強く再確認。
  「host-agent may perform adapter-local intake, framing, relay-spool persistence」を削除
  (spoolは送信側=アダプタ/取り込みクライアントの持ち物=D1)
- **0015 explicit host control plane**: host control plane契約はsudo級スコープに絞って存続。
  device-facing provider actions(command envelope)はhost-agent境界から外し、D4南向きディスパッチ+
  キュー5へ付け替え(キュー5確定までは「host-agentを経由しない」とだけ明記)。
  コア記述から「provider」語を除去

### グループB: mTLS不採用の波及(0017, 0021, 0033, 0038, 0042、+0025の一部)

共通原因: D3決定5「サーバー側TLS(自己署名+フィンガープリントピン留め)+operatorトークンに統一。
mTLS不採用・CA基盤を作らない」。

- **0017 http-json transport profiles**: remoteプロファイルの「mutually authenticated network transport」を
  「TLS(自己署名+ピン留め)+operator/per-deviceトークン」に書き換え。骨子(HTTP+JSON、UDS/remote分割)は維持
- **0021 security and authentication baseline**: 「split-device seams require mTLS」を置換。
  骨子(無認証リモート面禁止・route-family認可・秘密の非露出・監査必須)はR19/R14と一致し維持。
  「PKI未選定」記述を「CA基盤は作らない(確定)」に更新
- **0033 certificate lifecycle and rotation**: 中心決定(mTLS運用)は棄却だが骨子は転用可。
  「ゲートウェイ自己署名TLS証明書+operatorトークンのライフサイクル」として再構成:
  オーバーラップローテーション=ピン留めフィンガープリントの新旧並行受理期間、平文降格禁止=D1の明示opt-in、
  per-deviceトークンのハッシュ保存・失効、R22スナップショットの高機密扱い(D2§3.5)を追加
- **0038 explicit revocation and operator identity**: 失効(signed deny-list)とoperator安定IDの骨子は
  operatorトークン単位に置き換えて存続。90日ローテーションはトークンローテーションとして継続。
  「Alternative B(トークンのみ)rejected」の記述を撤回(採用済み方式に逆転したため)。
  D1の「AI operatorトークンは物理アクション権限へ昇格不可」を権限段階節に追記
- **0042 minimum deployment environment baseline**: LAN/IP割当/クロック健全性等の前提は維持。
  「集中証明書・クレデンシャル管理+その管理拠点をサイトに要求」条項を撤回し、
  「各ゲートウェイが自己署名で自己完結。トークン発行・失効もゲートウェイローカル(R19)。
  横断バックアップはD2§3.5の暗号化スナップショット」に書き換え

### グループC: D1/D4/D5の精密化反映(0009, 0011, 0013, 0016, 0024)

- **0009 pipeline separation**: 入力パイプラインの「adapter → collector → **normalizer** → bus」が
  D1「正規化はつなぐ側。コアは生バイト列を受けない」と矛盾。
  「ドライバ(デコード)+ランタイム(写像)→取り込みクライアント→コレクタ(R8)→R9(較正)→bus」に書き換え。
  コマンド側パイプラインの詳細はキュー5確定後に再定義と明記
- **0011 bounded adapter properties**: escape hatch(config_adapter_properties)はドライバ/ランタイム層限定と
  明文化し、取り込みクライアント(stable-intent契約側)に及ばないこと、promotion先=R5/R14を固定
- **0013 stable sensor series identity**: 骨子の大半はD5が明示継承(8状態・series_key構成規則・channel_index)。
  「イベントが安定series識別を運搬する」前提を「コレクタが台帳解決チェーンで導出(D5決定1)」に書き換え。
  「logical measurement identityの粒度」はD5で確定済み(subject_id=system_id)と追記
- **0016 collector ingress batch ack**: 骨子(バッチ+per-envelope ack、4語彙、envelope_id安定性)はD1と一致。
  3点反映: (1)「structurally invalid envelopes are quarantined」→「rejected(終端)」に修正
  (検疫の語は用語集定義=値域外/未登録キー/登録直後、に限定)、(2) acceptedの`disposition: durable|staged`追記、
  (3) dedupキー=(認証済み送信者, envelope_id)+ingest_dedupテーブル一本化を追記
- **0024 contract test authority**: 骨子(契約テストカタログ=フレームワーク選定前のゲート)はD1/D4の
  共有適合テストスイートで再確認。series-identityテストケースの主体を送信側からコレクタ側解決に付け替え、
  参照先をD1/D4/D5に更新。南向きテストカタログはキュー5確定まで存在しない旨を明記

### グループD: 実装実態との乖離(0025, 0039)

- **0025 first-wave implementation stack**: 3点が古い。
  (1) `sqlx`指定 → 実コードはrusqlite(D1のDbHandle記述もrusqlite前提)。rusqlite継続を明記、
  (2) コンポーネント分割(gateway-runtime/api/migrator)が実workspaceにもD4クレート計画
  (iotkit-ingest-contract/client新設、core/ledger)にも不一致 → 実際のクレートマップ参照に書き直し
  (host-agent/gatewayctl/migratorは「未実装の将来クレート」と明示)、
  (3) 「rustls for TLS and mTLS」→ mTLS削除
- **0039 sqlite single-writer batching profile**: (1) `synchronous=NORMAL`(D1明記)が欠落 → 追加、
  (2) 「10件or100msフラッシュ」をD1の位置づけ(group commitは伸びたら追加する将来拡張)に合わせ、
  Wave 0は即時コミット+メトリクス確認後に導入の段階付けに書き直し、
  (3) 本ADRの「batching」(writer-lane flush)とD1「エンベロープバッチ」(ワイヤ契約)は別概念と脚注、
  (4) ack=SQLiteコミット後(耐久点)とのクロスリファレンス+同一トランザクション制約(dedup+測定書き込み)の明記

### グループE: その他(0023, 0026, 0030, 0031)

- **0023 observability and recoverability baseline**: 骨子(構造化ログ・監査・バックアップ検証必須)は
  R12/R13/R19/R22と整合。D2§3.5の必須事項(スナップショット暗号化・専用退避チャネル・エポックフェンス・
  復旧runbookの旧機無効化ステップ)を検証項目に追記。「edge hardware」→「ゲートウェイ(RPi)」
- **0026 yokakit edge context**: タイトル含めedge語を全面置換。冒頭に「YokaKitは再設計凍結・
  リファレンス消費者(D3決定3)であり、出口契約(R10)上は特権なしの一消費者」を明示的に上書き。
  取り込み契約の具体化はキュー3確定待ちと追記
- **0030 bounded outbox retry policy**: 読み替え方針はD3で決定済み。
  「must not silently discard」→「宣言されたR17劣化契約の外での破棄禁止(契約外の黙示破棄禁止)」。
  劣化契約に従った統制済み破棄(間引き/要約化/最古削除、水位ヒステリシス)は正規動作であり
  構造化ログ(R12/R13)に残す。パージ可否はアーカイブ責任消費者ack(D2§1)を上位ゲートとする。
  ※出口契約全体への統合はキュー3で実施
- **0031 contract versioning baseline**(**新規発見** — 事前リスト外): 2つの穴。
  (1) D3決定2の「安定意図(stable-intent)」段階が0031の変更管理規律に存在しない →
  family版管理の正式な一段階として追加(外部消費者出現前は移行ノートのみ、出現後にADR-first厳格運用へ遷移)、
  (2) docs/redesign/decisions/配下の決定文書と正式ADRの権威関係・昇格ルールが未定義 →
  本文書冒頭の権威規則を恒久化し、決定文書=pre-ADR設計コンセンサス、昇格のタイミングと手順を0031に規定

### 統合済み(4本)— キュー3確定(D7-exit-contract 2026-07-03)で処置

確定的矛盾なし。以下の反映メモはD7の「ADR統合処置」節で具体化済み(0030の出口側=再送単位・
exhaustion可視化・no silent dropもD7で回収。0030の取り込み側ack語彙はD1処置済みのまま):

- **0028 yokakit publisher as projection adapter**: 事前に疑われた「YokaKit特権」は精査の結果**シロ**
  (本ADR自身が投影のコア語彙化を禁止しており、R10非特権原則を先取り)。統合時:
  record family「edge-runtime-status」の改称(これのみキュー3非依存で即時可)、検疫遷移annotation配送・
  エポック複合カーソル・アーカイブ責任消費者ack・最低保持フロアの組み込み、複数消費者共有の明記
- **0029 yokakit http push first wave**: push-first/publication_id/at-least-onceは用語集と方向一致。
  統合時: edge語置換、「本ADRは出口(R10)側でありD1の北向きバインディングとは別方向」の明記、
  アーカイブ責任消費者・R17劣化契約・0035 fan-outとの接続。
  **(D9改訂 2026-07-08)** 第一波バインディングはMQTT QoS1へ改訂(HTTPは追加候補に降格)。0029の却下理由
  2点(ブローカー中心の複雑性/request-response可視性)への応答はD9「ADR 0029への応答」参照
- **0032 yokakit cold-start recovery**: 原則(gateway側current stateが権威・bounded jobs・無制限履歴
  エクスポート不要)は「バッファであって倉庫ではない」と一致。統合時: 本ADRの「snapshot」を
  D2§3.5のR22スナップショット(高機密資産)と別語に改称(例: publication snapshot)、
  検疫遷移配送をbounded backfill対象に含めるか明記
- **0035 target-scoped upstream fan-out**: target_id+publication_id粒度の配送状態分離はD2の
  アーカイブ責任消費者ルールの一般形として整合。統合時: (epoch, seq)複合カーソルのtarget単位保持、
  アーカイブ責任消費者フラグの持ち方を定義

## 反映の運用

- ADR本文への反映は**各設計キューの確定時**に本文書の処置案どおり実施する(一括書き換えはしない)
- グループA/B/C/D/Eの大半は依存なしで反映可能だが、0005/0009/0015/0024の南向き言及部分はキュー5、
  0026/0030の出口契約部分と保留4本はキュー3の確定が前提
- 反映したADRは本文書の該当行に反映日を追記する
