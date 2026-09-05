# Changelog

All notable user-visible and operational changes to IoTKit are recorded here.
Product versions do not replace versioned API, MQTT, disk, snapshot, adapter,
configuration, or OKF format identifiers.

## [Unreleased]

- Edge Nodeの暗号化backup、fenced restore、`iotkit-core-recovery`、`nodectl`の`snapshot` / `backup` / `restore`を削除しました。復旧はTOML、SQLiteファイル、`pipelines.toml`の3点をコピーする手順です。NTP同期は必須です。
- Removed encrypted Edge Node backup, fenced restore, `iotkit-core-recovery`, and the `nodectl` `snapshot` / `backup` / `restore` commands. Recovery is now copying the TOML, the SQLite file, and `pipelines.toml`. NTP synchronization is required.
- 中央のIoTKit Edge（`edge/`）、そのcustody契約と旧Output Adapter契約、中央側の統合スクリプト、compose定義を削除しました。Edge Nodeは#232 の再設計でMQTT Output Adapter契約 v1により標準MQTT Brokerへ直接公開します。
- Removed the central IoTKit Edge (`edge/`), its custody contract and the old Output Adapter contract, the central integration scripts, and the compose definitions. Since the redesign in #232 the Edge Node publishes directly to a standard MQTT Broker under the MQTT Output Adapter contract v1.
- 試用profileをEdge Node + Mosquittoの構成に書き換えました。管理者passwordとConsoleはなくなり、`./scripts/iotkit trial up`が3本のpipelineをimportし、`./scripts/iotkit trial watch`でObservationとstatusを表示します。`iotkit.toml`の`console_bind` / `console_port`は受け付けません。
- Rewrote the trial profile around the Edge Node and Mosquitto. The administrator password and the Console are gone; `./scripts/iotkit trial up` imports three pipelines and `./scripts/iotkit trial watch` shows Observations and status. `console_bind` / `console_port` in `iotkit.toml` are no longer accepted.

## [0.4.0] - 2026-08-08

- iotkit.tomlからloopback限定のtrial profileを起動し、通常のInput Adapterと保管責任経路を通る照度三角波・接点状態矩形波のsampleを確認できるようにしました。安全なvalidate、up、down、reset手順も追加しました。
- Added a TOML-driven loopback-only trial profile with illuminance triangle-wave and contact-state square-wave samples through the normal Input Adapter and custody path, plus safe validate, up, down, and reset operations.
- 登録済みsensorのLive monitorを追加し、保存済みで有効なcumulative_counter ruleの画面開始後グラフとcurrent/statusを示します。Liveからcanonicalなsensor詳細へ移動できます。
- Added a Live monitor for registered sensors with page-open history and current/status for saved active cumulative_counter rules, with navigation to the canonical sensor-detail page.
- Historyとsensor-detail previewの選択sensor、graph、表示時刻を揃え、遅いpreview pollingが後続requestを妨げないようにしました。表示値の不要な小数末尾0も省略します。
- Aligned history and sensor-detail preview selection, charts, and display times; slow preview polling no longer starves later requests, and displayed values omit unnecessary trailing fractional zeroes.
- Semantic projectionを保持するraw history（既定4 Edge Nodeで合計400,000件）とは独立したbounded queue/workとして処理し、schema v10のsemantic-history indexを追加しました。
- Semantic projection now runs as bounded pending work independent of retained raw history (400,000 records in the default four-Edge-Node profile), with the schema v10 semantic-history index for history queries.
- 遅延したMQTT custody ACK、deferred ACK、idle後の新規recordを安全に処理し、pending ACKのretryとidle probeによってdeliveryを継続します。
- Improved MQTT custody handling for delayed and deferred acknowledgements and new records after idle, retrying pending ACKs and keeping delivery continuous with an idle probe.

## [0.3.0] - 2026-07-31

- Edge Nodeの暗号化backup、fail-closedなfenced restoreとhardware replacement、復旧権限による安全な再稼働、現場向け復旧手順を追加しました。backupの設定と保存先は現場要件に応じて任意に選べます。
- Added encrypted Edge Node backups, fail-closed fenced restore and hardware replacement, safe reactivation with recovery authority, and a field recovery guide. Backup configuration and storage location remain optional to suit each site.
- 証明書のhostname検証は、OpenSSLが不一致を表示しながら成功statusを返すhostでも不一致を拒否します。
- Certificate hostname validation now rejects mismatches even on hosts where OpenSSL reports a mismatch with a successful exit status.
- 公開repositoryで脆弱性の詳細を公開Issueへ書かずに報告できるprivate vulnerability reporting導線を追加しました。
- Added a private vulnerability reporting path so security details do not need to be disclosed in public issues.

## [0.2.0] - 2026-07-29

- センサー設定の実信号プレビューは、開いている通常ルールまたは異常検知ルールだけを追跡し、受信値と選択ルールの判定結果を分けて表示します。エラー時には別ルールの古い判定結果を残しません。
- The sensor-settings live preview now follows only the open measurement or alarm rule, separates received values from the selected rule outcome, and clears stale outcomes after errors.
- コンソールの主要8画面は960px以下でモバイルナビゲーションと積み上げ表示に切り替わり、画面全体の横スクロールを防ぎます。受信履歴と変更履歴の表は、必要な場合に表の中だけで横スクロールできます。
- The eight principal Console pages now switch to mobile navigation and stacked layouts at 960px and below without document-level horizontal scrolling. History and audit tables keep any necessary horizontal scrolling inside the table region.
- コンソールの外部出力画面は配信状態を優先して表示し、サマリー件数と宛先ごとの状態・対象数・最終送信・バックログを示します。技術的な詳細は折りたたみ式で、閲覧者は読み取り専用、狭い画面にも対応します。
- The Console external-output page now prioritizes delivery status, with summary counts plus each destination's state, targets, last send, and backlog. Technical details are collapsible; viewers remain read-only, and the layout supports narrow screens.

## [0.1.0] - 2026-07-27

- Initial public source release.
- Durable Edge Node collection and IoTKit Edge custody acknowledgement.
- Authenticated Console, semantic mapping, history, diagnostics, and backup.
- Durable generic MQTT JSON and Pinikiet output adapters.
