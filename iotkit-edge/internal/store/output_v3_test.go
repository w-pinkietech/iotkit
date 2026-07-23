package store

import (
	"context"
	"encoding/json"
	"path/filepath"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/outputadapter"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
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
		edgeapp.LocalCLIActor(),
		signalRef,
		"生産回数",
		semantics.RuleSpec{
			Kind: semantics.KindCumulativeCounter,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
			Trigger: semantics.TriggerTransition,
		},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	route, err := archive.ApplyPinikietRuleRoute(
		ctx,
		edgeapp.LocalCLIActor(),
		rule.ID,
		outputadapter.PinikietConfig{
			SourceID: "line-a",
			SensorID: "press",
			Kind:     outputadapter.PinikietProduction,
		},
	)
	if err != nil {
		t.Fatal(err)
	}
	updated, err := archive.UpdateSemanticRule(
		ctx,
		edgeapp.LocalCLIActor(),
		rule.ID,
		"良品数",
		semantics.RuleSpec{
			Kind: semantics.KindCumulativeCounter,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
			Trigger: semantics.TriggerTransition,
		},
		edgeapp.RevisionPrecondition{Expected: &rule.Revision},
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
	outputRoutes, err := archive.ListOutputRoutes(ctx)
	if err != nil || len(outputRoutes) != 1 ||
		outputRoutes[0].OldestPendingAt == nil {
		t.Fatalf("output routes=%#v err=%v", outputRoutes, err)
	}
	routes, err := archive.ListPinikietRuleRoutes(ctx)
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
	routes, err = archive.ListPinikietRuleRoutes(ctx)
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
		edgeapp.LocalCLIActor(),
		signalRef,
		"生産回数",
		semantics.RuleSpec{
			Kind: semantics.KindCumulativeCounter,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
			Trigger: semantics.TriggerTransition,
		},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
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
		edgeapp.LocalCLIActor(),
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

func TestOutputRouteTransformFailureDoesNotBlockOtherRoutesAndClears(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	ctx := context.Background()
	configuration, err := archive.GetSemanticConfiguration(ctx, signalRef)
	if err != nil {
		t.Fatal(err)
	}
	first, err := archive.CreateSemanticRule(
		ctx,
		edgeapp.LocalCLIActor(),
		signalRef,
		"補正値A",
		semantics.RuleSpec{Kind: semantics.KindNumeric},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	configuration, err = archive.GetSemanticConfiguration(ctx, signalRef)
	if err != nil {
		t.Fatal(err)
	}
	second, err := archive.CreateSemanticRule(
		ctx,
		edgeapp.LocalCLIActor(),
		signalRef,
		"補正値B",
		semantics.RuleSpec{Kind: semantics.KindNumeric},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	createRoute := func(ruleID, topic string) OutputRoute {
		t.Helper()
		config, encodeErr := outputadapter.EncodeGenericMQTTJSONConfig(
			outputadapter.GenericMQTTJSONConfig{Topic: topic},
		)
		if encodeErr != nil {
			t.Fatal(encodeErr)
		}
		route, createErr := archive.ApplyOutputRoute(
			ctx,
			edgeapp.LocalCLIActor(),
			ruleID,
			"iotkit.mqtt-json.v1",
			config,
		)
		if createErr != nil {
			t.Fatal(createErr)
		}
		return route
	}
	broken := createRoute(first.ID, "factory/line-a/a")
	if _, err := archive.db.Exec(`
		UPDATE output_routes SET config_schema_version = 99 WHERE route_id = ?
	`, broken.RouteID); err != nil {
		t.Fatal(err)
	}

	backlog := make([][]float64, 40)
	for index := range backlog {
		backlog[index] = []float64{float64(index)}
	}
	acceptSemanticBatch(
		t,
		archive,
		"edge-node-01",
		"epoch-a",
		2,
		backlog...,
	)
	if _, err := archive.ProjectSemanticRules(ctx, 1000); err != nil {
		t.Fatal(err)
	}
	healthy := createRoute(second.ID, "factory/line-a/b")
	acceptSemanticBatch(
		t,
		archive,
		"edge-node-01",
		"epoch-a",
		42,
		[]float64{24.8},
	)
	if _, err := archive.ProjectSemanticRules(ctx, 100); err != nil {
		t.Fatal(err)
	}
	count, enqueueErr := archive.EnqueueMultipleRuleOutputExports(ctx, 10)
	if enqueueErr == nil || count != 1 {
		t.Fatalf("enqueued=%d err=%v, want one healthy route and error", count, enqueueErr)
	}
	routes, err := archive.ListOutputRoutes(ctx)
	if err != nil {
		t.Fatal(err)
	}
	byID := make(map[string]OutputRoute, len(routes))
	for _, route := range routes {
		byID[route.RouteID] = route
	}
	if byID[broken.RouteID].LastTransformErrorCode != "config_version_mismatch" ||
		byID[broken.RouteID].LastTransformErrorAt == nil ||
		byID[healthy.RouteID].PendingCount != 1 ||
		byID[healthy.RouteID].LastTransformErrorCode != "" {
		t.Fatalf("routes after failure=%#v", routes)
	}

	continuedBacklog := make([][]float64, 20)
	for index := range continuedBacklog {
		continuedBacklog[index] = []float64{100 + float64(index)}
	}
	acceptSemanticBatch(
		t,
		archive,
		"edge-node-01",
		"epoch-a",
		43,
		continuedBacklog...,
	)
	if _, err := archive.ProjectSemanticRules(ctx, 1000); err != nil {
		t.Fatal(err)
	}
	if _, err := archive.db.Exec(`
		UPDATE output_routes SET config_schema_version = 1 WHERE route_id = ?
	`, broken.RouteID); err != nil {
		t.Fatal(err)
	}
	count, err = archive.EnqueueMultipleRuleOutputExports(ctx, 1)
	if err != nil || count != 1 {
		t.Fatalf("recovery enqueued=%d err=%v", count, err)
	}
	count, err = archive.EnqueueMultipleRuleOutputExports(ctx, 1)
	if err != nil || count != 1 {
		t.Fatalf("post-recovery fair enqueue=%d err=%v", count, err)
	}
	routes, err = archive.ListOutputRoutes(ctx)
	if err != nil {
		t.Fatal(err)
	}
	byID = make(map[string]OutputRoute, len(routes))
	for _, route := range routes {
		byID[route.RouteID] = route
	}
	if byID[broken.RouteID].LastTransformErrorCode != "" ||
		byID[broken.RouteID].LastTransformErrorAt != nil ||
		byID[broken.RouteID].LastTransformSuccessAt == nil ||
		byID[broken.RouteID].PendingCount != 1 ||
		byID[healthy.RouteID].PendingCount != 2 {
		t.Fatalf("routes after recovery=%#v", routes)
	}
}

func TestMultipleRuleStateAndOutputRemainExactlyOnceAcrossRestart(t *testing.T) {
	path := filepath.Join(t.TempDir(), "edge.db")
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
		edgeapp.LocalCLIActor(),
		signalRef,
		"生産回数",
		semantics.RuleSpec{
			Kind: semantics.KindCumulativeCounter,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
			Trigger: semantics.TriggerTransition,
		},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.ApplyPinikietRuleRoute(
		ctx,
		edgeapp.LocalCLIActor(),
		rule.ID,
		outputadapter.PinikietConfig{
			SourceID: "line-a",
			SensorID: "press",
			Kind:     outputadapter.PinikietProduction,
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
		edgeapp.LocalCLIActor(),
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
		edgeapp.LocalCLIActor(),
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
