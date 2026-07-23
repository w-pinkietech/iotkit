package store

import (
	"context"
	"encoding/json"
	"path/filepath"
	"strings"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/outputadapter"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
)

func TestV1TwoEdgeSemanticOutputJourneySurvivesRestart(t *testing.T) {
	path := filepath.Join(t.TempDir(), "edge.db")
	archive, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	for _, edgeNode := range []string{"edge-node-01", "edge-node-02"} {
		acceptSemanticBatch(t, archive, edgeNode, "epoch-a", 1, []float64{0})
	}
	if _, err := archive.ReconcileInventorySources(ctx, 100); err != nil {
		t.Fatal(err)
	}
	signals, err := archive.ListInventorySignals(ctx, 100, "")
	if err != nil || len(signals) != 2 {
		t.Fatalf("signals=%#v err=%v", signals, err)
	}
	for index, signal := range signals {
		definition, err := archive.ApplySemanticDefinition(
			ctx, edgeapp.LocalCLIActor(), signal.SignalRef,
			semantics.DefinitionSpec{
				Kind: semantics.KindBoolean, Scale: 1,
				Detector: semantics.Detector{
					Mode: semantics.DetectorBooleanHighActive,
				},
			}, edgeapp.RevisionPrecondition{},
		)
		if err != nil {
			t.Fatal(err)
		}
		_, err = archive.ApplyPinikietRoute(
			ctx, edgeapp.LocalCLIActor(), definition.ID,
			outputadapter.PinikietConfig{
				SourceID: "iotkit-01",
				SensorID: []string{"press-a", "press-b"}[index],
				Kind:     outputadapter.PinikietOnOff,
			},
		)
		if err != nil {
			t.Fatal(err)
		}
	}
	acceptSemanticBatch(t, archive, "edge-node-01", "epoch-a", 2, []float64{1})
	acceptSemanticBatch(t, archive, "edge-node-02", "epoch-a", 2, []float64{0})
	if _, err := archive.ProjectSemanticObservations(ctx, 100); err != nil {
		t.Fatal(err)
	}
	if _, err := archive.EnqueueOutputExports(ctx, 100); err != nil {
		t.Fatal(err)
	}
	pending, err := archive.ListPendingMQTTExports(ctx, 100)
	if err != nil || len(pending) != 2 {
		t.Fatalf("pending=%#v err=%v", pending, err)
	}
	for _, item := range pending {
		if !strings.HasPrefix(item.Topic, "pinikiet/v1/sources/iotkit-01/sensors/") {
			t.Fatalf("topic=%q", item.Topic)
		}
		var payload struct {
			SchemaVersion int    `json:"schema_version"`
			Kind          string `json:"kind"`
			Value         bool   `json:"value"`
		}
		if err := json.Unmarshal(item.PayloadJSON, &payload); err != nil {
			t.Fatal(err)
		}
		if payload.SchemaVersion != 1 || payload.Kind != "onoff" {
			t.Fatalf("payload=%s", item.PayloadJSON)
		}
	}
	if err := archive.Close(); err != nil {
		t.Fatal(err)
	}
	reopened, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer reopened.Close()
	pendingAfterRestart, err := reopened.ListPendingMQTTExports(ctx, 100)
	if err != nil || len(pendingAfterRestart) != 2 {
		t.Fatalf("pending after restart=%#v err=%v", pendingAfterRestart, err)
	}
	for _, item := range pendingAfterRestart {
		if err := reopened.MarkMQTTExportPublished(ctx, item.ExportID); err != nil {
			t.Fatal(err)
		}
	}
	remaining, err := reopened.ListPendingMQTTExports(ctx, 100)
	if err != nil || len(remaining) != 0 {
		t.Fatalf("remaining=%#v err=%v", remaining, err)
	}
}
