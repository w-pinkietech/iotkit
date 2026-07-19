package store

import (
	"context"
	"encoding/json"
	"path/filepath"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/outputadapter"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantics"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
)

func TestMultipleRuleOutputRouteSurvivesRuleRevisionAndPublishes(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	ctx := context.Background()
	configuration, err := archive.GetSemanticConfiguration(ctx, signalRef)
	if err != nil {
		t.Fatal(err)
	}
	rule, err := archive.CreateSemanticRule(
		ctx,
		siteapp.LocalCLIActor(),
		signalRef,
		"生産回数",
		semantics.RuleSpec{
			Kind: semantics.KindCumulativeCounter,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
			Trigger: semantics.TriggerTransition,
		},
		siteapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	route, err := archive.ApplyYokaKitRuleRoute(
		ctx,
		siteapp.LocalCLIActor(),
		rule.ID,
		outputadapter.YokaKitConfig{
			SourceID: "line-a",
			SignalID: "production",
			Kind:     outputadapter.YokaKitProduction,
		},
	)
	if err != nil {
		t.Fatal(err)
	}
	updated, err := archive.UpdateSemanticRule(
		ctx,
		siteapp.LocalCLIActor(),
		rule.ID,
		"良品数",
		semantics.RuleSpec{
			Kind: semantics.KindCumulativeCounter,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
			Trigger: semantics.TriggerTransition,
		},
		siteapp.RevisionPrecondition{Expected: &rule.Revision},
	)
	if err != nil || updated.ID != rule.ID {
		t.Fatalf("updated=%#v err=%v", updated, err)
	}
	acceptSemanticBatch(
		t,
		archive,
		"edge-node-01",
		"epoch-a",
		2,
		[]float64{0},
		[]float64{1},
	)
	if _, err := archive.ProjectSemanticRules(ctx, 100); err != nil {
		t.Fatal(err)
	}
	if count, err := archive.EnqueueMultipleRuleOutputExports(ctx, 100); err != nil ||
		count != 1 {
		t.Fatalf("enqueued=%d err=%v", count, err)
	}
	routes, err := archive.ListYokaKitRuleRoutes(ctx)
	if err != nil || len(routes) != 1 ||
		routes[0].RuleID != rule.ID ||
		routes[0].RouteID != route.RouteID ||
		routes[0].PendingCount != 1 {
		t.Fatalf("routes=%#v err=%v", routes, err)
	}
	pending, err := archive.ListPendingMQTTExports(ctx, 100)
	if err != nil || len(pending) != 1 ||
		pending[0].RouteID != route.RouteID {
		t.Fatalf("pending=%#v err=%v", pending, err)
	}
	if err := archive.MarkMQTTExportPublished(ctx, pending[0].ExportID); err != nil {
		t.Fatal(err)
	}
	routes, err = archive.ListYokaKitRuleRoutes(ctx)
	if err != nil || routes[0].PublishedCount != 1 {
		t.Fatalf("published routes=%#v err=%v", routes, err)
	}
}

func TestGenericOutputRouteUsesRegisteredAdapter(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	ctx := context.Background()
	configuration, err := archive.GetSemanticConfiguration(ctx, signalRef)
	if err != nil {
		t.Fatal(err)
	}
	rule, err := archive.CreateSemanticRule(
		ctx,
		siteapp.LocalCLIActor(),
		signalRef,
		"生産回数",
		semantics.RuleSpec{
			Kind: semantics.KindCumulativeCounter,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
			Trigger: semantics.TriggerTransition,
		},
		siteapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	config, err := outputadapter.EncodeGenericMQTTJSONConfig(
		outputadapter.GenericMQTTJSONConfig{
			Topic: "factory/line-a/production",
		},
	)
	if err != nil {
		t.Fatal(err)
	}
	route, err := archive.ApplyOutputRoute(
		ctx,
		siteapp.LocalCLIActor(),
		rule.ID,
		"iotkit.mqtt-json.v1",
		config,
	)
	if err != nil {
		t.Fatal(err)
	}
	if route.AdapterID != "iotkit.mqtt-json.v1" ||
		route.ConfigSchemaVersion != 1 ||
		string(route.Config) != string(config) {
		t.Fatalf("route = %#v", route)
	}
	routes, err := archive.ListOutputRoutes(ctx)
	if err != nil || len(routes) != 1 || routes[0].RouteID != route.RouteID {
		t.Fatalf("routes=%#v err=%v", routes, err)
	}
	acceptSemanticBatch(
		t,
		archive,
		"edge-node-01",
		"epoch-a",
		2,
		[]float64{0},
		[]float64{1},
	)
	if _, err := archive.ProjectSemanticRules(ctx, 100); err != nil {
		t.Fatal(err)
	}
	if count, err := archive.EnqueueMultipleRuleOutputExports(ctx, 100); err != nil ||
		count != 1 {
		t.Fatalf("enqueued=%d err=%v", count, err)
	}
	pending, err := archive.ListPendingMQTTExports(ctx, 100)
	if err != nil || len(pending) != 1 ||
		pending[0].Topic != "factory/line-a/production" {
		t.Fatalf("pending=%#v err=%v", pending, err)
	}
	var payload struct {
		Kind  outputadapter.ObservationKind `json:"kind"`
		Value int64                         `json:"value"`
	}
	if err := json.Unmarshal(pending[0].PayloadJSON, &payload); err != nil {
		t.Fatal(err)
	}
	if payload.Kind != outputadapter.KindCumulativeValue || payload.Value != 1 {
		t.Fatalf("payload=%#v raw=%s", payload, pending[0].PayloadJSON)
	}
}

func TestMultipleRuleStateAndOutputRemainExactlyOnceAcrossRestart(t *testing.T) {
	path := filepath.Join(t.TempDir(), "site.db")
	archive, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	signalRef := semanticV3Signal(t, archive)
	ctx := context.Background()
	configuration, err := archive.GetSemanticConfiguration(ctx, signalRef)
	if err != nil {
		t.Fatal(err)
	}
	rule, err := archive.CreateSemanticRule(
		ctx,
		siteapp.LocalCLIActor(),
		signalRef,
		"生産回数",
		semantics.RuleSpec{
			Kind: semantics.KindCumulativeCounter,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
			Trigger: semantics.TriggerTransition,
		},
		siteapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.ApplyYokaKitRuleRoute(
		ctx,
		siteapp.LocalCLIActor(),
		rule.ID,
		outputadapter.YokaKitConfig{
			SourceID: "line-a",
			SignalID: "production",
			Kind:     outputadapter.YokaKitProduction,
		},
	); err != nil {
		t.Fatal(err)
	}
	acceptSemanticBatch(
		t,
		archive,
		"edge-node-01",
		"epoch-a",
		2,
		[]float64{0},
		[]float64{1},
	)
	if _, err := archive.ProjectSemanticRules(ctx, 100); err != nil {
		t.Fatal(err)
	}
	reset, err := archive.RequestSemanticCounterReset(
		ctx,
		siteapp.LocalCLIActor(),
		rule.ID,
		"restart-reset",
	)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.ProjectSemanticRules(ctx, 100); err != nil {
		t.Fatal(err)
	}
	if count, err := archive.EnqueueMultipleRuleOutputExports(ctx, 100); err != nil ||
		count != 2 {
		t.Fatalf("first enqueue count=%d err=%v, want 2", count, err)
	}
	if count, err := archive.EnqueueMultipleRuleOutputExports(ctx, 100); err != nil ||
		count != 0 {
		t.Fatalf("repeat enqueue count=%d err=%v, want 0", count, err)
	}
	if err := archive.Close(); err != nil {
		t.Fatal(err)
	}

	archive, err = Open(path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = archive.Close() })
	configuration, err = archive.GetSemanticConfiguration(ctx, signalRef)
	if err != nil {
		t.Fatal(err)
	}
	if len(configuration.Rules) != 1 ||
		configuration.Rules[0].ID != rule.ID {
		t.Fatalf("reopened configuration = %#v", configuration)
	}
	reopenedReset, err := archive.RequestSemanticCounterReset(
		ctx,
		siteapp.LocalCLIActor(),
		rule.ID,
		reset.ID,
	)
	if err != nil {
		t.Fatal(err)
	}
	if reopenedReset.AppliedAt == nil {
		t.Fatalf("reopened reset is not applied: %#v", reopenedReset)
	}
	if count, err := archive.EnqueueMultipleRuleOutputExports(ctx, 100); err != nil ||
		count != 0 {
		t.Fatalf("enqueue after restart count=%d err=%v, want 0", count, err)
	}
	pending, err := archive.ListPendingMQTTExports(ctx, 100)
	if err != nil || len(pending) != 2 {
		t.Fatalf("pending after restart=%#v err=%v, want 2", pending, err)
	}
	for _, export := range pending {
		if err := archive.MarkMQTTExportPublished(ctx, export.ExportID); err != nil {
			t.Fatal(err)
		}
	}
	if err := archive.Close(); err != nil {
		t.Fatal(err)
	}

	archive, err = Open(path)
	if err != nil {
		t.Fatal(err)
	}
	pending, err = archive.ListPendingMQTTExports(ctx, 100)
	if err != nil || len(pending) != 0 {
		t.Fatalf("published exports after restart=%#v err=%v", pending, err)
	}
	var outboxRows int
	if err := archive.db.QueryRow(`
		SELECT count(*) FROM output_outbox_v3
	`).Scan(&outboxRows); err != nil {
		t.Fatal(err)
	}
	if outboxRows != 2 {
		t.Fatalf("durable outbox rows = %d, want 2", outboxRows)
	}
}
