package store

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"sort"
	"testing"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/contract"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
)

type capacityRegressionReport struct {
	Profile               Profile `json:"profile"`
	EdgeNodes             int     `json:"edge_nodes"`
	SensorsPerEdge        int     `json:"sensors_per_edge"`
	Records               int     `json:"records"`
	PayloadBytes          int     `json:"payload_bytes"`
	RecordsPerSecond      float64 `json:"records_per_second"`
	AcceptP99Millis       int64   `json:"accept_p99_millis"`
	HistoryQueryMillis    int64   `json:"history_query_millis"`
	BackupMillis          int64   `json:"backup_millis"`
	DatabaseBytes         int64   `json:"database_bytes"`
	PendingOutput         int64   `json:"pending_output"`
	ProjectionFailures    int64   `json:"projection_failures"`
	RegressionSmokePassed bool    `json:"regression_smoke_passed"`
}

func TestStorageCapacityRegressionSmoke(t *testing.T) {
	reportPath := os.Getenv("IOTKIT_CAPACITY_REPORT")
	if reportPath == "" {
		t.Skip("IOTKIT_CAPACITY_REPORT is not set")
	}
	archive := openTestStore(t)
	const edgeNodes = 4
	const sensorsPerEdge = 8
	const recordsPerEdge = 2_000
	const batchSize = 100
	latencies := make([]time.Duration, 0, edgeNodes*recordsPerEdge/batchSize)
	started := time.Now()
	payloadBytes := 0
	for edge := 0; edge < edgeNodes; edge++ {
		edgeNodeID := "capacity-edge-" + intToDecimal(edge+1)
		epoch := "capacity-epoch-1"
		for start := 1; start <= recordsPerEdge; start += batchSize {
			records := make([]json.RawMessage, 0, batchSize)
			for offset := 0; offset < batchSize; offset++ {
				sequence := int64(start + offset)
				encoded, err := json.Marshal(map[string]any{
					"family": "measurement", "schema_version": 1,
					"epoch": epoch, "pub_seq": sequence,
					"series_key": "00000000-0000-0000-0000-00000000000" + intToDecimal((offset%sensorsPerEdge)+1) + ":temperature_c:na:primary",
					"event_time": sequence * 1000, "values": []float64{20 + float64(offset%10)},
				})
				if err != nil {
					t.Fatal(err)
				}
				payloadBytes += len(encoded)
				records = append(records, encoded)
			}
			batch := contract.RecordBatch{
				SchemaVersion: 1, EdgeNodeID: edgeNodeID, LedgerEpoch: epoch,
				CursorStart: int64(start), CursorEnd: int64(start + batchSize - 1),
				Records: records,
			}
			batch.PublicationID = contract.PublicationID(
				batch.EdgeNodeID, batch.LedgerEpoch, batch.CursorStart, batch.CursorEnd,
			)
			acceptedAt := time.Now()
			if _, err := acceptBatchForTest(t, archive, batch); err != nil {
				t.Fatal(err)
			}
			latencies = append(latencies, time.Since(acceptedAt))
		}
	}
	duration := time.Since(started)
	sort.Slice(latencies, func(left, right int) bool { return latencies[left] < latencies[right] })
	p99 := latencies[(len(latencies)*99-1)/100]
	queryStarted := time.Now()
	if _, err := archive.ListRawRecords(context.Background(), 10_000); err != nil {
		t.Fatal(err)
	}
	queryDuration := time.Since(queryStarted)
	backupStarted := time.Now()
	if _, err := archive.ApplyEncryptedBackup(
		context.Background(), edgeapp.LocalCLIActor(),
		filepath.Join(t.TempDir(), "capacity.iotkit-backup"),
		"capacity-test-passphrase",
	); err != nil {
		t.Fatal(err)
	}
	backupDuration := time.Since(backupStarted)
	status, err := archive.GetStorageStatus(context.Background(), 90)
	if err != nil {
		t.Fatal(err)
	}
	report := capacityRegressionReport{
		Profile: status.Profile, EdgeNodes: edgeNodes, SensorsPerEdge: sensorsPerEdge,
		Records: edgeNodes * recordsPerEdge, PayloadBytes: payloadBytes,
		RecordsPerSecond: float64(edgeNodes*recordsPerEdge) / duration.Seconds(),
		AcceptP99Millis:  p99.Milliseconds(), HistoryQueryMillis: queryDuration.Milliseconds(),
		BackupMillis: backupDuration.Milliseconds(), DatabaseBytes: status.DatabaseBytes,
		PendingOutput: status.PendingOutputCount, ProjectionFailures: status.ProjectionFailureCount,
	}
	report.RegressionSmokePassed = p99 < 10*time.Second && queryDuration < 10*time.Second &&
		backupDuration < 60*time.Second && status.RawRecordCount == int64(report.Records)
	encoded, err := json.MarshalIndent(report, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(reportPath, append(encoded, '\n'), 0o600); err != nil {
		t.Fatal(err)
	}
	if !report.RegressionSmokePassed {
		t.Fatalf("capacity regression smoke failed: %s", encoded)
	}
}
