package mqttsite

import (
	"context"
	"errors"
	"io"
	"log/slog"
	"strings"
	"testing"
	"time"

	mqtt "github.com/eclipse/paho.mqtt.golang"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/store"
)

type fakePublishToken struct {
	done chan struct{}
	err  error
}

func (token *fakePublishToken) Done() <-chan struct{} { return token.done }
func (token *fakePublishToken) Error() error          { return token.err }

type fakeExportQueue struct {
	pending      []store.PendingMQTTExport
	marked       int
	markedIDs    []string
	markErrs     map[string]error
	projected    int
	reconciled   int
	reconcileErr error
	enqueued     int
	listed       int
}

type fakeActivationQueue struct {
	fakeExportQueue
	commands       []store.ActivationCommand
	attemptedIDs   []string
	attemptedAt    []int64
	listErr        error
	markAttemptErr error
}

type fakeMultipleRuleQueue struct {
	fakeExportQueue
	v2Projected int
	v3Projected int
	v2Outputs   int
	v3Outputs   int
}

type fakeV3OnlyQueue struct {
	fakeExportQueue
	projected int
	enqueued  int
}

func (queue *fakeV3OnlyQueue) ProjectSemanticRules(
	context.Context,
	int,
) (int, error) {
	queue.projected++
	return 0, nil
}

func (queue *fakeV3OnlyQueue) EnqueueMultipleRuleOutputExports(
	context.Context,
	int,
) (int, error) {
	queue.enqueued++
	return 0, nil
}

func (queue *fakeMultipleRuleQueue) ProjectSemanticObservations(
	context.Context,
	int,
) (int, error) {
	queue.v2Projected++
	return 0, nil
}

func (queue *fakeMultipleRuleQueue) ProjectSemanticRules(
	context.Context,
	int,
) (int, error) {
	queue.v3Projected++
	return 0, nil
}

func (queue *fakeMultipleRuleQueue) EnqueueOutputExports(
	context.Context,
	int,
) (int, error) {
	queue.v2Outputs++
	return 0, nil
}

func (queue *fakeMultipleRuleQueue) EnqueueMultipleRuleOutputExports(
	context.Context,
	int,
) (int, error) {
	queue.v3Outputs++
	return 0, nil
}

func (queue *fakeActivationQueue) ListPendingActivationCommands(
	context.Context,
	int,
) ([]store.ActivationCommand, error) {
	return queue.commands, queue.listErr
}

func (queue *fakeActivationQueue) MarkActivationCommandAttempt(
	_ context.Context,
	activationID string,
	at int64,
) error {
	queue.attemptedIDs = append(queue.attemptedIDs, activationID)
	queue.attemptedAt = append(queue.attemptedAt, at)
	return queue.markAttemptErr
}

func (queue *fakeExportQueue) ReconcileInventorySources(context.Context, int) (int, error) {
	queue.reconciled++
	return 0, queue.reconcileErr
}

func (queue *fakeExportQueue) ListPendingMQTTExports(context.Context, int) ([]store.PendingMQTTExport, error) {
	queue.listed++
	return queue.pending, nil
}

func (queue *fakeExportQueue) MarkMQTTExportPublished(_ context.Context, exportID string) error {
	if err := queue.markErrs[exportID]; err != nil {
		return err
	}
	queue.marked++
	queue.markedIDs = append(queue.markedIDs, exportID)
	return nil
}

func (queue *fakeExportQueue) ProjectSemanticEvents(context.Context, int) (int, error) {
	queue.projected++
	return 0, nil
}

func (queue *fakeExportQueue) EnqueueMQTTExports(context.Context, int) (int, error) {
	queue.enqueued++
	return 0, nil
}

func TestRecordsTopicFilterUsesEdgeNodes(t *testing.T) {
	if recordsTopicFilter != "iotkit/v1/edge-nodes/+/records" {
		t.Fatalf("records topic filter = %q", recordsTopicFilter)
	}
}

func TestDescriptorTopicFilterUsesEdgeNodes(t *testing.T) {
	if descriptorsTopicFilter != "iotkit/v1/edge-nodes/+/descriptors" {
		t.Fatalf("descriptors topic filter = %q", descriptorsTopicFilter)
	}
}

func TestActivationResultTopicFilterUsesEdgeNodes(t *testing.T) {
	if activationResultTopicFilter != "iotkit/v1/edge-nodes/+/activation/result" {
		t.Fatalf("activation result topic filter = %q", activationResultTopicFilter)
	}
}

func TestConvergenceUsesMultipleRuleProjectionWhenAvailable(t *testing.T) {
	queue := &fakeMultipleRuleQueue{}
	convergeSite(
		context.Background(),
		queue,
		slog.New(slog.NewTextHandler(io.Discard, nil)),
	)
	if queue.v3Projected != 1 || queue.v2Projected != 0 {
		t.Fatalf(
			"v3 projected=%d, v2 projected=%d",
			queue.v3Projected,
			queue.v2Projected,
		)
	}
	if queue.v3Outputs != 1 {
		t.Fatalf("v3 output enqueue=%d", queue.v3Outputs)
	}
}

func TestConvergenceDoesNotRequireLegacyGenericInterfaceForV3(t *testing.T) {
	queue := &fakeV3OnlyQueue{}
	convergeSite(
		context.Background(),
		queue,
		slog.New(slog.NewTextHandler(io.Discard, nil)),
	)
	if queue.projected != 1 || queue.enqueued != 1 {
		t.Fatalf(
			"v3-only projected=%d enqueued=%d",
			queue.projected,
			queue.enqueued,
		)
	}
}

func TestActivationCommandPublishRetriesUntilResultAndNeverCompletesOnPUBACK(t *testing.T) {
	queue := &fakeActivationQueue{commands: []store.ActivationCommand{{
		ActivationID: "act-0123456789abcdef0123456789abcdef",
		Topic:        "iotkit/v1/edge-nodes/edge-node-01/activation/request",
		PayloadJSON:  []byte(`{"schema_version":1}`),
	}}}
	var topics []string

	for range 2 {
		err := publishPendingActivationCommands(
			context.Background(),
			queue,
			func(topic string, qos byte, retained bool, _ []byte) error {
				topics = append(topics, topic)
				if qos != 1 || retained {
					t.Fatalf("activation publish qos=%d retained=%t", qos, retained)
				}
				return nil
			},
		)
		if err != nil {
			t.Fatal(err)
		}
	}

	if len(topics) != 2 || len(queue.attemptedIDs) != 2 {
		t.Fatalf("topics=%v attempts=%v", topics, queue.attemptedIDs)
	}
	if queue.commands[0].ActivationID != "act-0123456789abcdef0123456789abcdef" {
		t.Fatal("PUBACK unexpectedly completed the durable command")
	}
}

func TestActivationCommandPublishFailureDoesNotRecordAttempt(t *testing.T) {
	queue := &fakeActivationQueue{commands: []store.ActivationCommand{{
		ActivationID: "act-0123456789abcdef0123456789abcdef",
		Topic:        "iotkit/v1/edge-nodes/edge-node-01/activation/request",
		PayloadJSON:  []byte(`{"schema_version":1}`),
	}}}

	err := publishPendingActivationCommands(
		context.Background(),
		queue,
		func(string, byte, bool, []byte) error {
			return errors.New("broker unavailable")
		},
	)

	if err == nil {
		t.Fatal("activation publish failure was hidden")
	}
	if len(queue.attemptedIDs) != 0 {
		t.Fatalf("attempts = %v", queue.attemptedIDs)
	}
}

func TestPahoPublishUsesWholeOperationWriteTimeout(t *testing.T) {
	options := newClientOptions(ClientConfig{
		BrokerURL: "tcp://127.0.0.1:1883",
		ClientID:  "site-test",
		Username:  "site-test",
		Password:  "not-a-real-secret",
	})
	reader := mqtt.NewOptionsReader(options)
	if got := reader.WriteTimeout(); got != publishAcknowledgementTTL {
		t.Fatalf("Paho write timeout = %s, want %s", got, publishAcknowledgementTTL)
	}
}

func TestPahoPublishExpiredWholeOperationDeadlineDoesNotWaitAgain(t *testing.T) {
	token := &fakePublishToken{done: make(chan struct{})}
	err := waitForPublishCompletion(context.Background(), token, time.Now().Add(-time.Millisecond))
	if err == nil || !strings.Contains(err.Error(), "timed out") {
		t.Fatalf("expired publish deadline error = %v", err)
	}
}

func TestPahoPublishWholeOperationDeadlineIncludesBlockedPublishCall(t *testing.T) {
	releasePublish := make(chan struct{})
	publishStarted := make(chan struct{})
	publishReleased := make(chan struct{})
	deadline := time.Now().Add(30 * time.Millisecond)
	started := time.Now()
	err := publishWithDeadline(context.Background(), func() publishToken {
		close(publishStarted)
		<-releasePublish
		close(publishReleased)
		done := make(chan struct{})
		close(done)
		return &fakePublishToken{done: done}
	}, deadline)
	elapsed := time.Since(started)

	select {
	case <-publishStarted:
	default:
		t.Fatal("publish call did not start")
	}
	if err == nil || !strings.Contains(err.Error(), "timed out") {
		t.Fatalf("blocked publish error = %v, want timeout", err)
	}
	if elapsed > 300*time.Millisecond {
		t.Fatalf("blocked publish returned after %s, want bounded by injected deadline", elapsed)
	}
	close(releasePublish)
	select {
	case <-publishReleased:
	case <-time.After(time.Second):
		t.Fatal("blocked fake publish did not release")
	}
}

func TestExportLoopMarksPublishedOnlyAfterPublishSuccess(t *testing.T) {
	queue := &fakeExportQueue{pending: []store.PendingMQTTExport{{
		ExportID:    "export-01",
		Topic:       "iotkit/v1/application/production-pulses",
		QoS:         1,
		PayloadJSON: []byte(`{"schema_version":1}`),
	}}}

	err := publishPending(context.Background(), queue, func(string, byte, []byte) error {
		return errors.New("broker unavailable")
	})
	if err == nil {
		t.Fatal("publishPending succeeded despite publish failure")
	}
	if queue.marked != 0 {
		t.Fatalf("failed publish marked %d exports", queue.marked)
	}

	err = publishPending(context.Background(), queue, func(_ string, qos byte, _ []byte) error {
		if qos != 1 {
			t.Fatalf("publish QoS = %d", qos)
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if queue.marked != 1 {
		t.Fatalf("successful publish marked %d exports", queue.marked)
	}
}

func TestExportLoopIsolatesFailedRouteAndPreservesOtherRoutes(t *testing.T) {
	queue := &fakeExportQueue{pending: []store.PendingMQTTExport{
		{ExportID: "route-a-1", RouteID: "route-a", Topic: "factory/a", QoS: 1},
		{ExportID: "route-b-1", RouteID: "route-b", Topic: "factory/b", QoS: 1},
		{ExportID: "route-a-2", RouteID: "route-a", Topic: "factory/a", QoS: 1},
	}}
	published := make([]string, 0)
	err := publishPending(context.Background(), queue, func(topic string, _ byte, _ []byte) error {
		published = append(published, topic)
		if topic == "factory/a" {
			return errors.New("route A unavailable")
		}
		return nil
	})
	if err == nil || !strings.Contains(err.Error(), "route-a-1") {
		t.Fatalf("publishPending error = %v, want route A failure", err)
	}
	if got, want := strings.Join(published, ","), "factory/a,factory/b"; got != want {
		t.Fatalf("published topics = %s, want %s", got, want)
	}
	if got, want := strings.Join(queue.markedIDs, ","), "route-b-1"; got != want {
		t.Fatalf("marked exports = %s, want %s", got, want)
	}
}

func TestExportLoopAggregatesQoSAndMarkFailuresAcrossRoutes(t *testing.T) {
	queue := &fakeExportQueue{
		pending: []store.PendingMQTTExport{
			{ExportID: "route-a-1", RouteID: "route-a", Topic: "factory/a", QoS: 0},
			{ExportID: "route-b-1", RouteID: "route-b", Topic: "factory/b", QoS: 1},
			{ExportID: "route-c-1", RouteID: "route-c", Topic: "factory/c", QoS: 1},
			{ExportID: "route-a-2", RouteID: "route-a", Topic: "factory/a", QoS: 1},
			{ExportID: "route-b-2", RouteID: "route-b", Topic: "factory/b", QoS: 1},
		},
		markErrs: map[string]error{"route-b-1": errors.New("mark unavailable")},
	}
	published := make([]string, 0)
	err := publishPending(context.Background(), queue, func(topic string, _ byte, _ []byte) error {
		published = append(published, topic)
		return nil
	})
	if err == nil || !strings.Contains(err.Error(), "route-a-1") || !strings.Contains(err.Error(), "route-b-1") {
		t.Fatalf("publishPending error = %v, want aggregated QoS and mark failures", err)
	}
	if got, want := strings.Join(published, ","), "factory/b,factory/c"; got != want {
		t.Fatalf("published topics = %s, want %s", got, want)
	}
	if got, want := strings.Join(queue.markedIDs, ","), "route-c-1"; got != want {
		t.Fatalf("marked exports = %s, want %s", got, want)
	}
}

func TestConvergenceStopsBeforeWorkWhenContextIsCanceled(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	queue := &fakeExportQueue{}
	published := 0
	convergeExports(ctx, queue, func(string, byte, []byte) error {
		published++
		return nil
	}, slog.New(slog.NewTextHandler(io.Discard, nil)))

	if queue.reconciled != 0 || queue.projected != 0 || queue.enqueued != 0 || queue.listed != 0 || published != 0 || queue.marked != 0 {
		t.Fatalf("canceled convergence performed work: %+v, published=%d", queue, published)
	}
}

func TestConvergenceReconcilesInventoryWithoutBlockingSemanticWork(t *testing.T) {
	queue := &fakeExportQueue{reconcileErr: errors.New("inventory unavailable")}
	convergeExports(
		context.Background(),
		queue,
		func(string, byte, []byte) error { return nil },
		slog.New(slog.NewTextHandler(io.Discard, nil)),
	)

	if queue.reconciled != 1 || queue.projected != 1 || queue.enqueued != 1 {
		t.Fatalf("convergence calls = reconcile %d, project %d, enqueue %d",
			queue.reconciled, queue.projected, queue.enqueued)
	}
}
