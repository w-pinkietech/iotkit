# Changelog

All notable user-visible and operational changes to IoTKit are recorded here.
Product versions do not replace versioned API, MQTT, disk, snapshot, adapter,
configuration, or OKF format identifiers.

## [Unreleased]

- コンソールの外部出力画面は配信状態を優先して表示し、サマリー件数と宛先ごとの状態・対象数・最終送信・バックログを示します。技術的な詳細は折りたたみ式で、閲覧者は読み取り専用、狭い画面にも対応します。
- The Console external-output page now prioritizes delivery status, with summary counts plus each destination's state, targets, last send, and backlog. Technical details are collapsible; viewers remain read-only, and the layout supports narrow screens.

## [0.1.0] - 2026-07-27

- Initial public source release.
- Durable Edge Node collection and IoTKit Edge custody acknowledgement.
- Authenticated Console, semantic mapping, history, diagnostics, and backup.
- Durable generic MQTT JSON and Pinikiet output adapters.
