package store

import (
	"bytes"
	"context"
	"encoding/json"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/applicationcontract"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantic"
)

func TestPutMQTTRouteRejectsInvalidTopicAndUnknownMapping(t *testing.T) {
	store := openTestStore(t)
	mapping := putSemanticMapping(t, store, semantic.TriggerActiveSample, 1)

	for _, topic := range []string{"", " ", "/factory/pulses", "factory/pulses/", "factory/+/pulses", "factory/#"} {
		t.Run(topic, func(t *testing.T) {
			if _, err := store.PutMQTTRoute(context.Background(), mapping.ID, topic); err == nil {
				t.Fatalf("PutMQTTRoute accepted topic %q", topic)
			}
		})
	}
	if _, err := store.PutMQTTRoute(context.Background(), "sm-unknown", "factory/pulses"); err == nil {
		t.Fatal("PutMQTTRoute accepted an unknown mapping")
	}
	if got := store.testCount(t, "mqtt_routes"); got != 0 {
		t.Fatalf("route count = %d, want 0", got)
	}
}

func TestRouteExportsOnlyEventsCreatedAfterRouteAndFansOut(t *testing.T) {
	store := openTestStore(t)
	mapping := putSemanticMapping(t, store, semantic.TriggerActiveSample, 1)
	acceptContactBatch(t, store, "edge-node-01", "epoch-a", 1, []float64{1})
	projectSemanticEvents(t, store)

	routeA := putMQTTRoute(t, store, mapping.ID, "factory/a/production-pulses")
	routeB := putMQTTRoute(t, store, mapping.ID, "factory/b/production-pulses")
	acceptContactBatch(t, store, "edge-node-01", "epoch-a", 2, []float64{1})
	projectSemanticEvents(t, store)

	if got, err := store.EnqueueMQTTExports(context.Background(), 100); err != nil {
		t.Fatal(err)
	} else if got != 2 {
		t.Fatalf("enqueued = %d, want 2", got)
	}
	if got, err := store.EnqueueMQTTExports(context.Background(), 100); err != nil {
		t.Fatal(err)
	} else if got != 0 {
		t.Fatalf("idempotent enqueue inserted %d rows, want 0", got)
	}

	pending := listPendingMQTTExports(t, store)
	if len(pending) != 2 {
		t.Fatalf("pending = %#v, want two exports", pending)
	}
	gotTopics := []string{pending[0].Topic, pending[1].Topic}
	wantTopics := []string{routeA.Topic, routeB.Topic}
	if !reflect.DeepEqual(gotTopics, wantTopics) {
		t.Fatalf("topics = %v, want %v", gotTopics, wantTopics)
	}
	for _, export := range pending {
		assertProductionPulsePayload(t, export.PayloadJSON, mapping.ID, 1, 2)
	}
}

func TestRouteBoundaryUsesEventRowIDAcrossMappingRevisions(t *testing.T) {
	store := openTestStore(t)
	first := putSemanticMapping(t, store, semantic.TriggerActiveSample, 1)
	acceptContactBatch(t, store, "edge-node-01", "epoch-a", 1, []float64{1})
	projectSemanticEvents(t, store)
	putMQTTRoute(t, store, first.ID, "factory/production-pulses")

	second := putSemanticMapping(t, store, semantic.TriggerActiveSample, 1)
	acceptContactBatch(t, store, "edge-node-01", "epoch-a", 2, []float64{1})
	projectSemanticEvents(t, store)
	if got, err := store.EnqueueMQTTExports(context.Background(), 100); err != nil {
		t.Fatal(err)
	} else if got != 1 {
		t.Fatalf("enqueued = %d, want revision-reset event", got)
	}

	pending := listPendingMQTTExports(t, store)
	if len(pending) != 1 {
		t.Fatalf("pending = %#v, want one export", pending)
	}
	assertProductionPulsePayload(t, pending[0].PayloadJSON, second.ID, second.Revision, 1)
}

func TestEnqueueMQTTExportsReturnsZeroWhenTransactionRollsBack(t *testing.T) {
	store := openTestStore(t)
	mapping := putSemanticMapping(t, store, semantic.TriggerActiveSample, 1)
	putMQTTRoute(t, store, mapping.ID, "factory/production-pulses")
	acceptContactBatch(t, store, "edge-node-01", "epoch-a", 1, []float64{1})
	projectSemanticEvents(t, store)

	if _, err := store.db.Exec(`
		INSERT INTO semantic_events (
			event_id, mapping_id, mapping_revision, event_sequence, meaning,
			edge_node_id, ledger_epoch, source_pub_seq, source_series_key,
			occurred_at, created_at
		) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
	`, "invalid-event", mapping.ID, mapping.Revision, 2, semantic.MeaningProductionPulse,
		"edge-node-01", "epoch-injected", 2, contactSeries, -1, 1); err != nil {
		t.Fatal(err)
	}

	enqueued, err := store.EnqueueMQTTExports(context.Background(), 100)
	if err == nil {
		t.Fatal("EnqueueMQTTExports accepted an invalid semantic event")
	}
	if enqueued != 0 {
		t.Fatalf("enqueued = %d after rollback, want 0", enqueued)
	}
	if got := store.testCount(t, "mqtt_export_outbox"); got != 0 {
		t.Fatalf("outbox rows = %d after rollback, want 0", got)
	}
}

func TestPendingMQTTExportRemainsUntilPublishedAndListIsReadOnly(t *testing.T) {
	path := filepath.Join(t.TempDir(), "site.db")
	store, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = store.Close() })
	mapping := putSemanticMapping(t, store, semantic.TriggerActiveSample, 1)
	putMQTTRoute(t, store, mapping.ID, "factory/production-pulses")
	acceptContactBatch(t, store, "edge-node-01", "epoch-a", 1, []float64{1})
	projectSemanticEvents(t, store)
	if _, err := store.EnqueueMQTTExports(context.Background(), 100); err != nil {
		t.Fatal(err)
	}
	if err := store.Close(); err != nil {
		t.Fatal(err)
	}
	store, err = Open(path)
	if err != nil {
		t.Fatal(err)
	}

	first := listPendingMQTTExports(t, store)
	if len(first) != 1 || first[0].Attempts != 0 {
		t.Fatalf("first pending = %#v, want one unattempted export", first)
	}
	second := listPendingMQTTExports(t, store)
	if len(second) != 1 || second[0].ExportID != first[0].ExportID || second[0].Attempts != 0 {
		t.Fatalf("second pending = %#v, want same unmodified export", second)
	}
	if err := store.MarkMQTTExportPublished(context.Background(), first[0].ExportID); err != nil {
		t.Fatal(err)
	}
	var firstPublishedAt int64
	if err := store.db.QueryRow(`
		SELECT published_at FROM mqtt_export_outbox WHERE export_id = ?
	`, first[0].ExportID).Scan(&firstPublishedAt); err != nil {
		t.Fatal(err)
	}
	if err := store.MarkMQTTExportPublished(context.Background(), first[0].ExportID); err != nil {
		t.Fatalf("idempotent publish mark failed: %v", err)
	}
	if pending := listPendingMQTTExports(t, store); len(pending) != 0 {
		t.Fatalf("pending after publish = %#v, want none", pending)
	}

	var attempts int64
	var publishedAt int64
	if err := store.db.QueryRow(`
		SELECT attempts, published_at FROM mqtt_export_outbox WHERE export_id = ?
	`, first[0].ExportID).Scan(&attempts, &publishedAt); err != nil {
		t.Fatal(err)
	}
	if attempts != 0 || publishedAt != firstPublishedAt {
		t.Fatalf("stored attempts/published_at = %d/%d, want 0/%d", attempts, publishedAt, firstPublishedAt)
	}
}

func TestListPendingMQTTExportsPreservesSameRouteEventOrder(t *testing.T) {
	store := openTestStore(t)
	mapping := putSemanticMapping(t, store, semantic.TriggerActiveSample, 1)
	putMQTTRoute(t, store, mapping.ID, "factory/production-pulses")
	acceptContactBatch(t, store, "edge-node-01", "epoch-a", 1, []float64{1}, []float64{1})
	projectSemanticEvents(t, store)
	if _, err := store.EnqueueMQTTExports(context.Background(), 100); err != nil {
		t.Fatal(err)
	}
	events := listSemanticEvents(t, store)
	if len(events) != 2 {
		t.Fatalf("events = %#v, want two", events)
	}
	if _, err := store.db.Exec(`
		UPDATE mqtt_export_outbox
		SET created_at = CASE event_id WHEN ? THEN 200 ELSE 100 END
	`, events[0].EventID); err != nil {
		t.Fatal(err)
	}

	pending := listPendingMQTTExports(t, store)
	if len(pending) != 2 || pending[0].EventID != events[0].EventID || pending[1].EventID != events[1].EventID {
		t.Fatalf("pending event order = %#v, want semantic event row order", pending)
	}
}

func TestListPendingMQTTExportsFairlyIncludesIndependentRouteWithinLimit(t *testing.T) {
	store := openTestStore(t)
	mapping := putSemanticMapping(t, store, semantic.TriggerActiveSample, 1)
	routeA := putMQTTRoute(t, store, mapping.ID, "factory/a/production-pulses")
	olderSamples := make([][]float64, 257)
	for index := range olderSamples {
		olderSamples[index] = []float64{1}
	}
	acceptContactBatch(t, store, "edge-node-01", "epoch-a", 1, olderSamples[:256]...)
	acceptContactBatch(t, store, "edge-node-01", "epoch-a", 257, olderSamples[256])
	if _, err := store.ProjectSemanticEvents(context.Background(), 1_000); err != nil {
		t.Fatal(err)
	}
	routeB := putMQTTRoute(t, store, mapping.ID, "factory/b/production-pulses")
	acceptContactBatch(t, store, "edge-node-01", "epoch-a", 258, []float64{1})
	if _, err := store.ProjectSemanticEvents(context.Background(), 1_000); err != nil {
		t.Fatal(err)
	}
	if _, err := store.EnqueueMQTTExports(context.Background(), 1_000); err != nil {
		t.Fatal(err)
	}

	pending, err := store.ListPendingMQTTExports(context.Background(), 256)
	if err != nil {
		t.Fatal(err)
	}
	if len(pending) != 256 {
		t.Fatalf("pending count = %d, want 256", len(pending))
	}
	foundRouteB := false
	var previousASequence int64
	for _, export := range pending {
		switch export.RouteID {
		case routeA.RouteID:
			var payload applicationcontract.ProductionPulseV1
			if err := json.Unmarshal(export.PayloadJSON, &payload); err != nil {
				t.Fatal(err)
			}
			if payload.EventSequence != previousASequence+1 {
				t.Fatalf("route A event sequence = %d after %d, want in-order pending events", payload.EventSequence, previousASequence)
			}
			previousASequence = payload.EventSequence
		case routeB.RouteID:
			foundRouteB = true
		default:
			t.Fatalf("unexpected route %q", export.RouteID)
		}
	}
	if !foundRouteB {
		t.Fatal("newer independent route B was starved by route A fetch window")
	}
}

func projectSemanticEvents(t *testing.T, store *Store) {
	t.Helper()
	if _, err := store.ProjectSemanticEvents(context.Background(), 100); err != nil {
		t.Fatal(err)
	}
}

func putMQTTRoute(t *testing.T, store *Store, mappingID, topic string) MQTTRoute {
	t.Helper()
	route, err := store.PutMQTTRoute(context.Background(), mappingID, topic)
	if err != nil {
		t.Fatal(err)
	}
	return route
}

func listPendingMQTTExports(t *testing.T, store *Store) []PendingMQTTExport {
	t.Helper()
	pending, err := store.ListPendingMQTTExports(context.Background(), 100)
	if err != nil {
		t.Fatal(err)
	}
	return pending
}

func assertProductionPulsePayload(
	t *testing.T,
	payload json.RawMessage,
	mappingID string,
	mappingRevision, eventSequence int64,
) {
	t.Helper()
	var event applicationcontract.ProductionPulseV1
	if err := json.Unmarshal(payload, &event); err != nil {
		t.Fatalf("decode payload %s: %v", payload, err)
	}
	if err := event.Validate(); err != nil {
		t.Fatalf("invalid payload %s: %v", payload, err)
	}
	if event.MappingID != mappingID || event.MappingRevision != mappingRevision ||
		event.EventSequence != eventSequence || event.Count != eventSequence {
		t.Fatalf("payload = %#v, want mapping %s revision %d sequence/count %d", event, mappingID, mappingRevision, eventSequence)
	}
	deterministic, err := json.Marshal(event)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(payload, deterministic) {
		t.Fatalf("payload is not deterministic struct JSON: got %s want %s", payload, deterministic)
	}
}
