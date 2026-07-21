package store

import (
	"context"
	"database/sql"
	"strconv"
	"time"
)

const (
	defaultSensorStaleAfter = 5 * time.Minute
	defaultOutputStaleAfter = 5 * time.Minute
	maxDiagnosticIssues     = 500
)

type DiagnosticState string

const (
	DiagnosticHealthy   DiagnosticState = "healthy"
	DiagnosticAttention DiagnosticState = "attention"
	DiagnosticCritical  DiagnosticState = "critical"
)

type DiagnosticIssue struct {
	Code        string `json:"code"`
	Severity    string `json:"severity"`
	Component   string `json:"component"`
	ResourceRef string `json:"resource_ref,omitempty"`
	Summary     string `json:"summary"`
	Detail      string `json:"detail"`
	ObservedAt  *int64 `json:"observed_at,omitempty"`
}

type DiagnosticReport struct {
	GeneratedAt int64             `json:"generated_at"`
	State       DiagnosticState   `json:"state"`
	Issues      []DiagnosticIssue `json:"issues"`
	Truncated   bool              `json:"truncated"`
	Limitations []string          `json:"limitations"`
}

func (store *Store) GetDiagnostics(
	ctx context.Context,
	warningPercent int,
	now time.Time,
) (DiagnosticReport, error) {
	report := DiagnosticReport{
		GeneratedAt: now.UnixMilli(),
		State:       DiagnosticHealthy,
		Issues:      make([]DiagnosticIssue, 0),
		Limitations: []string{
			"IoTKit Edgeは現在、Input Adapterプロセス固有のhealthを受信しないため、収集ノード停止とAdapter停止を単独では判別できません。",
			"Broker接続失敗は再接続処理のlogと未受信・未配送の事実を組み合わせて確認します。",
		},
	}
	appendIssue := func(issue DiagnosticIssue) {
		if len(report.Issues) >= maxDiagnosticIssues {
			report.Truncated = true
			return
		}
		report.Issues = append(report.Issues, issue)
		switch issue.Severity {
		case "critical":
			report.State = DiagnosticCritical
		case "warning":
			if report.State == DiagnosticHealthy {
				report.State = DiagnosticAttention
			}
		}
	}

	storage, err := store.GetStorageStatus(ctx, warningPercent)
	if err != nil {
		return DiagnosticReport{}, err
	}
	switch storage.State {
	case StorageCritical:
		appendIssue(DiagnosticIssue{Code: "edge_storage_critical", Severity: "critical", Component: "edge_storage", Summary: "IoTKit Edgeの保存容量が残りわずかです", Detail: "未配送データは削除せず、導入担当者が容量とバックアップを確認してください。"})
	case StorageWarning:
		appendIssue(DiagnosticIssue{Code: "edge_storage_warning", Severity: "warning", Component: "edge_storage", Summary: "IoTKit Edgeの保存容量が少なくなっています", Detail: "警告水位へ到達しました。バックアップと保存方針を確認してください。"})
	case StorageUnavailable:
		appendIssue(DiagnosticIssue{Code: "edge_storage_unavailable", Severity: "warning", Component: "edge_storage", Summary: "hostの空き容量を確認できません", Detail: "DB自体の件数とhostのfilesystemを導入担当者が確認してください。"})
	}
	if storage.LastBackupAt == nil {
		appendIssue(DiagnosticIssue{Code: "edge_backup_missing", Severity: "warning", Component: "edge_backup", Summary: "検証済みバックアップがまだありません", Detail: "IoTKit Edgeのホストで暗号化バックアップを作成してください。"})
	}
	if storage.ProjectionFailureCount > 0 {
		appendIssue(DiagnosticIssue{Code: "semantic_projection_failed", Severity: "warning", Component: "semantic_projection", Summary: "意味付けに失敗した受信データがあります", Detail: "ルール設定と変換失敗件数を確認してください。"})
	}

	edgeNodeRows, err := store.db.QueryContext(ctx, `
		SELECT activation.edge_node_ref, activation.state,
			MAX(raw.received_at)
		FROM edge_node_activations AS activation
		LEFT JOIN raw_records AS raw
			ON raw.edge_node_id = activation.edge_node_id
		GROUP BY activation.edge_node_ref
		ORDER BY activation.edge_node_ref
	`)
	if err != nil {
		return DiagnosticReport{}, err
	}
	for edgeNodeRows.Next() {
		var edgeNodeRef, state string
		var lastRaw sql.NullInt64
		if err := edgeNodeRows.Scan(&edgeNodeRef, &state, &lastRaw); err != nil {
			_ = edgeNodeRows.Close()
			return DiagnosticReport{}, err
		}
		switch state {
		case "discovered":
			appendIssue(DiagnosticIssue{Code: "edge_node_activation_required", Severity: "warning", Component: "edge_node", ResourceRef: edgeNodeRef, Summary: "未登録の収集ノードがあります", Detail: "内容を確認してからIoTKit Edgeへ登録してください。"})
		case "activating":
			appendIssue(DiagnosticIssue{Code: "edge_node_activation_pending", Severity: "warning", Component: "edge_node", ResourceRef: edgeNodeRef, Summary: "収集ノードの登録が完了していません", Detail: "Broker通信と収集ノードの登録応答を確認してください。"})
		case "recovery_hold":
			appendIssue(DiagnosticIssue{Code: "edge_node_recovery_hold", Severity: "critical", Component: "edge_node", ResourceRef: edgeNodeRef, Summary: "収集ノードが復旧確認待ちです", Detail: "データ世代または復元後の欠番を確認するまで受信確認を進めません。"})
		case "active":
			if !lastRaw.Valid {
				appendIssue(DiagnosticIssue{Code: "edge_node_data_never_received", Severity: "warning", Component: "edge_node", ResourceRef: edgeNodeRef, Summary: "登録済みの収集ノードからデータをまだ受信していません", Detail: "センサー、Input Adapter、収集ノード、Brokerの順に確認してください。"})
			} else if now.UnixMilli()-lastRaw.Int64 > int64(defaultSensorStaleAfter/time.Millisecond) {
				value := lastRaw.Int64
				appendIssue(DiagnosticIssue{Code: "edge_node_data_stale", Severity: "warning", Component: "edge_node", ResourceRef: edgeNodeRef, Summary: "収集ノードからの受信が止まっています", Detail: "この事実だけではセンサー停止、Adapter停止、収集ノード停止、Broker断を区別できません。", ObservedAt: &value})
			}
		}
	}
	if err := edgeNodeRows.Err(); err != nil {
		_ = edgeNodeRows.Close()
		return DiagnosticReport{}, err
	}
	if err := edgeNodeRows.Close(); err != nil {
		return DiagnosticReport{}, err
	}

	signalRows, err := store.db.QueryContext(ctx, `
		SELECT signal_ref, last_received_at
		FROM edge_signals
		ORDER BY signal_ref
	`)
	if err != nil {
		return DiagnosticReport{}, err
	}
	for signalRows.Next() {
		var signalRef string
		var lastReceived sql.NullInt64
		if err := signalRows.Scan(&signalRef, &lastReceived); err != nil {
			_ = signalRows.Close()
			return DiagnosticReport{}, err
		}
		if !lastReceived.Valid {
			appendIssue(DiagnosticIssue{Code: "sensor_never_received", Severity: "warning", Component: "sensor", ResourceRef: signalRef, Summary: "未受信のセンサーがあります", Detail: "信号設定の前に実際の値が届くことを確認してください。"})
		} else if now.UnixMilli()-lastReceived.Int64 > int64(defaultSensorStaleAfter/time.Millisecond) {
			value := lastReceived.Int64
			appendIssue(DiagnosticIssue{Code: "sensor_data_stale", Severity: "warning", Component: "sensor", ResourceRef: signalRef, Summary: "センサーの値が古くなっています", Detail: "センサーと取得元の収集ノードを確認してください。", ObservedAt: &value})
		}
	}
	if err := signalRows.Err(); err != nil {
		_ = signalRows.Close()
		return DiagnosticReport{}, err
	}
	if err := signalRows.Close(); err != nil {
		return DiagnosticReport{}, err
	}

	var pendingOutput int64
	var oldestPending sql.NullInt64
	if err := store.db.QueryRowContext(ctx, `
		SELECT count(*), MIN(created_at)
		FROM output_outbox_v3 WHERE published_at IS NULL
	`).Scan(&pendingOutput, &oldestPending); err != nil {
		return DiagnosticReport{}, err
	}
	if pendingOutput > 0 && oldestPending.Valid && now.UnixMilli()-oldestPending.Int64 > int64(defaultOutputStaleAfter/time.Millisecond) {
		value := oldestPending.Int64
		appendIssue(DiagnosticIssue{Code: "output_delivery_stale", Severity: "warning", Component: "output", Summary: "外部出力が滞留しています", Detail: "外部Brokerの接続・認証・証明書を導入担当者が確認してください。未配送データは保持されています。", ObservedAt: &value})
	}
	var outputTransformFailures int64
	if err := store.db.QueryRowContext(ctx, `
		SELECT count(*) FROM output_routes
		WHERE lifecycle_state = 'active'
			AND last_transform_error_at IS NOT NULL
			AND (
				last_transform_success_at IS NULL
				OR last_transform_error_at > last_transform_success_at
			)
	`).Scan(&outputTransformFailures); err != nil {
		return DiagnosticReport{}, err
	}
	if outputTransformFailures > 0 {
		appendIssue(DiagnosticIssue{Code: "output_transform_failed", Severity: "warning", Component: "output_adapter", Summary: "外部出力のpayload変換に失敗しています", Detail: "Output Adapterの設定と対象ルールを確認してください。rawと意味付け済みデータは保持されています。"})
	}

	recoveryRows, err := store.db.QueryContext(ctx, `
		SELECT check_state.edge_node_id, check_state.ledger_epoch,
			check_state.backup_accepted_through, check_state.observed_cursor_start,
			check_state.updated_at
		FROM edge_restore_cursor_checks AS check_state
		WHERE check_state.state = 'recovery_required'
		ORDER BY check_state.updated_at, check_state.edge_node_id
	`)
	if err != nil {
		return DiagnosticReport{}, err
	}
	for recoveryRows.Next() {
		var edgeNodeID, epoch string
		var accepted, incoming, observedAt int64
		if err := recoveryRows.Scan(&edgeNodeID, &epoch, &accepted, &incoming, &observedAt); err != nil {
			_ = recoveryRows.Close()
			return DiagnosticReport{}, err
		}
		value := observedAt
		appendIssue(DiagnosticIssue{
			Code: "archive_recovery_required", Severity: "critical", Component: "edge_restore",
			ResourceRef: edgeNodeID, Summary: "復元後のIoTKit Edgeに欠けている可能性のあるデータがあります",
			Detail:     "ledger " + epoch + " の cursor " + formatInt(accepted+1) + " から " + formatInt(incoming-1) + " を確認してください。",
			ObservedAt: &value,
		})
	}
	if err := recoveryRows.Err(); err != nil {
		_ = recoveryRows.Close()
		return DiagnosticReport{}, err
	}
	if err := recoveryRows.Close(); err != nil {
		return DiagnosticReport{}, err
	}
	return report, nil
}

func formatInt(value int64) string {
	return strconv.FormatInt(value, 10)
}
