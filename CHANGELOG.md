# Changelog

All notable user-visible and operational changes to IoTKit are recorded here.
Product versions do not replace versioned API, MQTT, disk, snapshot, adapter,
configuration, or OKF format identifiers.

## [Unreleased]

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
