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
	pending   []store.PendingMQTTExport
	marked    int
	projected int
	enqueued  int
	listed    int
}

func (queue *fakeExportQueue) ListPendingMQTTExports(context.Context, int) ([]store.PendingMQTTExport, error) {
	queue.listed++
	return queue.pending, nil
}

func (queue *fakeExportQueue) MarkMQTTExportPublished(context.Context, string) error {
	queue.marked++
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

func TestConvergenceStopsBeforeWorkWhenContextIsCanceled(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	queue := &fakeExportQueue{}
	published := 0
	convergeExports(ctx, queue, func(string, byte, []byte) error {
		published++
		return nil
	}, slog.New(slog.NewTextHandler(io.Discard, nil)))

	if queue.projected != 0 || queue.enqueued != 0 || queue.listed != 0 || published != 0 || queue.marked != 0 {
		t.Fatalf("canceled convergence performed work: %+v, published=%d", queue, published)
	}
}
