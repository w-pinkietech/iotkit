//go:build integration

package mqttedge

import (
	"context"
	"encoding/json"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	mqtt "github.com/eclipse/paho.mqtt.golang"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/contract"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/outputadapter"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/store"
)

func TestMQTTOutputAdaptersConvergeAcrossBrokerRestart(t *testing.T) {
	runOutputAdapterBrokerJourney(t)
}

type observedMQTTMessage struct {
	topic    string
	payload  []byte
	retained bool
}

func runOutputAdapterBrokerJourney(t *testing.T) {
	t.Helper()
	brokerURL := requireEnv(t, "IOTKIT_TEST_OUTPUT_BROKER_URL")
	controlDir := requireEnv(t, "IOTKIT_TEST_OUTPUT_CONTROL_DIR")
	outputPassword := readTestPassword(t, "IOTKIT_TEST_OUTPUT_PASSWORD_FILE")
	observerPassword := readTestPassword(t, "IOTKIT_TEST_OUTPUT_OBSERVER_PASSWORD_FILE")

	archive, err := openOutputTestStore(controlDir)
	if err != nil {
		t.Fatal(err)
	}
	defer archive.Close()
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	genericRouteID, pinikietRouteID, sourceID := prepareOutputRoutes(t, archive)
	enqueueOutputTransition(t, archive, 2)
	pending, err := archive.ListPendingMQTTExports(ctx, 100)
	if err != nil || len(pending) < 2 {
		t.Fatalf("initial pending exports = %#v, %v", pending, err)
	}

	messages := make(chan observedMQTTMessage, 32)
	observer := connectOutputTestClient(
		t, brokerURL, "iotkit-output-e2e-observer", "observer", observerPassword,
	)
	defer observer.Disconnect(250)
	subscribeOutputTestTopics(t, observer, messages)

	runErrors := make(chan error, 1)
	go func() {
		runErrors <- RunOutput(ctx, ClientConfig{
			BrokerURL: brokerURL, ClientID: "iotkit-output-e2e-publisher",
			Username: "edge-output", Password: outputPassword, AllowInsecure: true,
		}, archive, slog.Default())
	}()

	waitForInitialAdapterMessages(t, messages)
	waitForNoPendingExports(t, archive, 20*time.Second)
	initialRoutes := outputRoutePublishedCounts(t, archive)
	if initialRoutes[genericRouteID] < 1 || initialRoutes[pinikietRouteID] < 1 {
		t.Fatalf("initial published counts = %#v", initialRoutes)
	}
	assertRetainedPinikietStatus(t, brokerURL, observerPassword, sourceID)

	writeTestMarker(t, controlDir, "ready")
	waitForTestMarker(t, controlDir, "broker-down", 30*time.Second)

	enqueueOutputTransition(t, archive, 4)
	pending, err = archive.ListPendingMQTTExports(ctx, 100)
	if err != nil || len(pending) < 2 {
		t.Fatalf("outage pending exports = %#v, %v", pending, err)
	}
	writeTestMarker(t, controlDir, "pending")

	waitForNoPendingExports(t, archive, 45*time.Second)
	recoveredRoutes := outputRoutePublishedCounts(t, archive)
	if recoveredRoutes[genericRouteID] <= initialRoutes[genericRouteID] ||
		recoveredRoutes[pinikietRouteID] <= initialRoutes[pinikietRouteID] {
		t.Fatalf(
			"published counts did not advance after restart: initial=%#v recovered=%#v",
			initialRoutes,
			recoveredRoutes,
		)
	}
	assertWrongSourcePublishDenied(
		t, brokerURL, outputPassword, observerPassword,
	)
	assertOutputCredentialCannotSubscribe(
		t, brokerURL, outputPassword, sourceID,
	)

	cancel()
	select {
	case err := <-runErrors:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("MQTT output runner did not stop")
	}
}

func openOutputTestStore(controlDir string) (*store.Store, error) {
	if postgresDSN := os.Getenv("IOTKIT_TEST_OUTPUT_POSTGRES_DSN"); postgresDSN != "" {
		return store.OpenWithOptions(store.OpenOptions{
			Profile:     store.ProfilePostgres,
			PostgresDSN: postgresDSN,
		})
	}
	return store.Open(filepath.Join(controlDir, "edge.db"))
}

func prepareOutputRoutes(t *testing.T, archive *store.Store) (string, string, string) {
	t.Helper()
	ctx := context.Background()
	descriptorPayload, err := os.ReadFile(filepath.Join(
		"..", "..", "..", "testdata", "egress", "v2", "descriptor-snapshot.json",
	))
	if err != nil {
		t.Fatal(err)
	}
	snapshot, err := contract.DecodeDescriptorSnapshot(descriptorPayload)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.ApplyDescriptorSnapshot(ctx, snapshot); err != nil {
		t.Fatal(err)
	}
	edgeNodes, err := archive.ListEdgeNodes(ctx)
	if err != nil || len(edgeNodes) != 1 {
		t.Fatalf("discovered edgeNodes = %#v, %v", edgeNodes, err)
	}
	expected := edgeNodes[0].Revision
	if _, err := archive.RequestEdgeNodeActivation(
		ctx,
		edgeapp.LocalCLIActor(),
		edgeNodes[0].EdgeNodeRef,
		edgeapp.RevisionPrecondition{Expected: &expected},
	); err != nil {
		t.Fatal(err)
	}
	commands, err := archive.ListPendingActivationCommands(ctx, 10)
	if err != nil || len(commands) != 1 {
		t.Fatalf("activation commands = %#v, %v", commands, err)
	}
	request, err := contract.DecodeActivationRequest(commands[0].PayloadJSON)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.ApplyActivationResult(ctx, contract.ActivationResult{
		SchemaVersion:            1,
		ActivationID:             request.ActivationID,
		EdgeID:                   request.EdgeID,
		EdgeNodeID:               request.EdgeNodeID,
		LedgerEpoch:              request.ExpectedLedgerEpoch,
		Status:                   "applied",
		DiscardThroughReadingSeq: 0,
		FirstPublicationSeq:      1,
		AppliedAt:                request.IssuedAt + 1,
	}); err != nil {
		t.Fatal(err)
	}
	acceptOutputBatch(t, archive, 1, []float64{0})
	if _, err := archive.ReconcileInventorySources(ctx, 100); err != nil {
		t.Fatal(err)
	}
	signals, err := archive.ListInventorySignals(ctx, 10, "")
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals = %#v, %v", signals, err)
	}
	configuration, err := archive.GetSemanticConfiguration(ctx, signals[0].SignalRef)
	if err != nil {
		t.Fatal(err)
	}
	counter, err := archive.CreateSemanticRule(
		ctx,
		edgeapp.LocalCLIActor(),
		signals[0].SignalRef,
		"累積値",
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
	configuration, err = archive.GetSemanticConfiguration(ctx, signals[0].SignalRef)
	if err != nil {
		t.Fatal(err)
	}
	state, err := archive.CreateSemanticRule(
		ctx,
		edgeapp.LocalCLIActor(),
		signals[0].SignalRef,
		"運転状態",
		semantics.RuleSpec{
			Kind: semantics.KindBoolean,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
		},
		edgeapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	genericProfile, err := archive.ActivateExportProfile(
		ctx,
		edgeapp.LocalCLIActor(),
		"IoTKit common MQTT",
		"iotkit.mqtt-json.v1",
	)
	if err != nil {
		t.Fatal(err)
	}
	pinikietProfile, err := archive.ActivateExportProfile(
		ctx,
		edgeapp.LocalCLIActor(),
		"Pinikiet",
		"pinikiet.mqtt.v1",
	)
	if err != nil {
		t.Fatal(err)
	}
	for _, binding := range pinikietProfile.Bindings {
		if binding.RuleID != state.ID {
			continue
		}
		configured, err := archive.ConfigurePinikietBooleanBinding(
			ctx,
			edgeapp.LocalCLIActor(),
			binding.BindingID,
			"onoff",
			binding.Revision,
		)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := archive.StartPreparedOutputBinding(
			ctx,
			edgeapp.LocalCLIActor(),
			configured.BindingID,
			configured.Revision,
		); err != nil {
			t.Fatal(err)
		}
	}
	routes, err := archive.ListOutputRoutes(ctx)
	if err != nil {
		t.Fatal(err)
	}
	var genericRouteID, pinikietRouteID string
	for _, route := range routes {
		switch {
		case route.RuleID == counter.ID &&
			route.AdapterID == genericProfile.AdapterID:
			genericRouteID = route.RouteID
		case route.RuleID == state.ID &&
			route.AdapterID == pinikietProfile.AdapterID:
			pinikietRouteID = route.RouteID
		}
	}
	if genericRouteID == "" || pinikietRouteID == "" {
		t.Fatalf("profile routes not found: %#v", routes)
	}
	return genericRouteID, pinikietRouteID, pinikietProfile.Bindings[0].SourceID
}

func assertWrongSourcePublishDenied(
	t *testing.T,
	brokerURL string,
	outputPassword string,
	observerPassword string,
) {
	t.Helper()
	const wrongTopic = "iotkit/v1/sources/" +
		"edge-00000000000000000000000000000000/" +
		"signals/sig-00000000000000000000000000000000/observations"
	received := make(chan struct{}, 1)
	observer := connectOutputTestClient(
		t,
		brokerURL,
		"iotkit-output-e2e-wrong-source-observer",
		"observer",
		observerPassword,
	)
	defer observer.Disconnect(250)
	if token := observer.Subscribe(
		wrongTopic,
		1,
		func(mqtt.Client, mqtt.Message) { received <- struct{}{} },
	); !token.WaitTimeout(10 * time.Second) {
		t.Fatal("wrong-source observer subscribe timeout")
	} else if err := token.Error(); err != nil {
		t.Fatal(err)
	}
	options := mqtt.NewClientOptions().
		AddBroker(brokerURL).
		SetClientID("iotkit-output-e2e-wrong-source").
		SetUsername("edge-output").
		SetPassword(outputPassword).
		SetAutoReconnect(false)
	client := mqtt.NewClient(options)
	if token := client.Connect(); !token.WaitTimeout(10 * time.Second) {
		t.Fatal("wrong-source publisher connect timeout")
	} else if err := token.Error(); err != nil {
		t.Fatal(err)
	}
	defer client.Disconnect(250)
	token := client.Publish(
		wrongTopic,
		1,
		false,
		`{"schema_version":1}`,
	)
	if !token.WaitTimeout(10 * time.Second) {
		t.Fatal("wrong-source publish did not receive an ACL decision")
	}
	select {
	case <-received:
		t.Fatal("Broker ACL accepted a topic for another Edge source")
	case <-time.After(750 * time.Millisecond):
	}
}

func assertOutputCredentialCannotSubscribe(
	t *testing.T,
	brokerURL string,
	outputPassword string,
	sourceID string,
) {
	t.Helper()
	topic := "iotkit/v1/sources/" + sourceID +
		"/signals/sig-00000000000000000000000000000000/observations"
	received := make(chan struct{}, 1)
	subscriber := connectOutputTestClient(
		t,
		brokerURL,
		"iotkit-output-e2e-no-subscribe",
		"edge-output",
		outputPassword,
	)
	defer subscriber.Disconnect(250)
	token := subscriber.Subscribe(
		topic,
		1,
		func(mqtt.Client, mqtt.Message) { received <- struct{}{} },
	)
	if !token.WaitTimeout(10 * time.Second) {
		t.Fatal("output credential subscribe did not receive an ACL decision")
	}
	if token.Error() != nil {
		return
	}

	publisher := connectOutputTestClient(
		t,
		brokerURL,
		"iotkit-output-e2e-no-subscribe-publisher",
		"edge-output",
		outputPassword,
	)
	defer publisher.Disconnect(250)
	published := publisher.Publish(
		topic,
		1,
		false,
		`{"schema_version":1}`,
	)
	if !published.WaitTimeout(10 * time.Second) {
		t.Fatal("authorized output publish timeout")
	}
	if err := published.Error(); err != nil {
		t.Fatal(err)
	}
	select {
	case <-received:
		t.Fatal("Broker ACL allowed the output credential to subscribe")
	case <-time.After(750 * time.Millisecond):
	}
}

func acceptOutputBatch(
	t *testing.T,
	archive *store.Store,
	start int64,
	values ...[]float64,
) {
	t.Helper()
	const (
		edgeNodeID = "edge-node-01"
		epoch      = "epoch-01"
		seriesKey  = "018f0000-0000-7000-8000-000000000001:contact_state:na:primary"
	)
	records := make([]json.RawMessage, 0, len(values))
	for index, value := range values {
		pubSeq := start + int64(index)
		encoded, err := json.Marshal(map[string]any{
			"family":            "measurement",
			"schema_version":    1,
			"epoch":             epoch,
			"pub_seq":           pubSeq,
			"series_key":        seriesKey,
			"values":            value,
			"event_time":        pubSeq * 1_000,
			"event_time_source": "received_at",
			"received_at":       pubSeq * 1_000,
			"device_time":       nil,
			"time_source":       "edge_node",
			"time_quality":      "unsynced",
		})
		if err != nil {
			t.Fatal(err)
		}
		records = append(records, encoded)
	}
	end := start + int64(len(records)) - 1
	_, err := archive.AcceptBatch(context.Background(), contract.RecordBatch{
		SchemaVersion: 1,
		EdgeNodeID:    edgeNodeID,
		LedgerEpoch:   epoch,
		PublicationID: contract.PublicationID(edgeNodeID, epoch, start, end),
		CursorStart:   start,
		CursorEnd:     end,
		Records:       records,
	})
	if err != nil {
		t.Fatal(err)
	}
}

func enqueueOutputTransition(t *testing.T, archive *store.Store, start int64) {
	t.Helper()
	acceptOutputBatch(t, archive, start, []float64{0}, []float64{1})
	if _, err := archive.ProjectSemanticRules(context.Background(), 100); err != nil {
		t.Fatal(err)
	}
	if count, err := archive.EnqueueMultipleRuleOutputExports(
		context.Background(),
		100,
	); err != nil || count < 2 {
		t.Fatalf("enqueued exports = %d, %v", count, err)
	}
}

func connectOutputTestClient(
	t *testing.T,
	brokerURL string,
	clientID string,
	username string,
	password string,
) mqtt.Client {
	t.Helper()
	options := mqtt.NewClientOptions().
		AddBroker(brokerURL).
		SetClientID(clientID).
		SetUsername(username).
		SetPassword(password).
		SetCleanSession(false).
		SetAutoReconnect(true)
	client := mqtt.NewClient(options)
	if token := client.Connect(); !token.WaitTimeout(10 * time.Second) {
		t.Fatal("MQTT observer connect timeout")
	} else if err := token.Error(); err != nil {
		t.Fatal(err)
	}
	return client
}

func subscribeOutputTestTopics(
	t *testing.T,
	client mqtt.Client,
	messages chan<- observedMQTTMessage,
) {
	t.Helper()
	handler := func(_ mqtt.Client, message mqtt.Message) {
		messages <- observedMQTTMessage{
			topic:    message.Topic(),
			payload:  append([]byte(nil), message.Payload()...),
			retained: message.Retained(),
		}
	}
	token := client.SubscribeMultiple(map[string]byte{
		"iotkit/v1/sources/+/signals/+/observations":   1,
		"pinikiet/v1/sources/+/sensors/+/observations": 1,
		"pinikiet/v1/sources/+/status":                 1,
	}, handler)
	if !token.WaitTimeout(10 * time.Second) {
		t.Fatal("MQTT observer subscribe timeout")
	} else if err := token.Error(); err != nil {
		t.Fatal(err)
	}
}

func waitForInitialAdapterMessages(
	t *testing.T,
	messages <-chan observedMQTTMessage,
) {
	t.Helper()
	var genericSeen, pinikietSeen bool
	deadline := time.NewTimer(20 * time.Second)
	defer deadline.Stop()
	for !genericSeen || !pinikietSeen {
		select {
		case message := <-messages:
			var payload struct {
				SchemaVersion int             `json:"schema_version"`
				Kind          string          `json:"kind"`
				Value         json.RawMessage `json:"value"`
			}
			if err := json.Unmarshal(message.payload, &payload); err != nil {
				t.Fatalf("decode %q payload: %v", message.topic, err)
			}
			switch {
			case strings.HasPrefix(message.topic, "iotkit/v1/sources/"):
				if payload.SchemaVersion != 1 {
					t.Fatalf("generic payload = %s", message.payload)
				}
				if payload.Kind == string(outputadapter.KindCumulativeValue) {
					genericSeen = true
				}
			case strings.HasPrefix(message.topic, "pinikiet/v1/sources/") &&
				strings.Contains(message.topic, "/sensors/"):
				if payload.SchemaVersion != 1 {
					t.Fatalf("Pinikiet payload = %s", message.payload)
				}
				if payload.Kind == string(outputadapter.PinikietOnOff) {
					pinikietSeen = true
				}
			}
		case <-deadline.C:
			t.Fatalf(
				"adapter MQTT messages timed out: generic=%t pinikiet=%t",
				genericSeen,
				pinikietSeen,
			)
		}
	}
}

func assertRetainedPinikietStatus(
	t *testing.T,
	brokerURL string,
	observerPassword string,
	sourceID string,
) {
	t.Helper()
	client := connectOutputTestClient(
		t,
		brokerURL,
		"iotkit-output-e2e-late-status-observer",
		"observer",
		observerPassword,
	)
	defer client.Disconnect(250)
	statuses := make(chan observedMQTTMessage, 1)
	token := client.Subscribe(
		"pinikiet/v1/sources/"+sourceID+"/status",
		1,
		func(_ mqtt.Client, message mqtt.Message) {
			statuses <- observedMQTTMessage{
				topic:    message.Topic(),
				payload:  append([]byte(nil), message.Payload()...),
				retained: message.Retained(),
			}
		},
	)
	if !token.WaitTimeout(10 * time.Second) {
		t.Fatal("late status subscribe timeout")
	} else if err := token.Error(); err != nil {
		t.Fatal(err)
	}
	select {
	case status := <-statuses:
		if !status.retained ||
			!strings.Contains(string(status.payload), `"state":"online"`) {
			t.Fatalf("retained Pinikiet status = %#v", status)
		}
	case <-time.After(10 * time.Second):
		t.Fatal("retained Pinikiet status timeout")
	}
}

func outputRoutePublishedCounts(
	t *testing.T,
	archive *store.Store,
) map[string]int64 {
	t.Helper()
	routes, err := archive.ListOutputRoutes(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	counts := make(map[string]int64, len(routes))
	for _, route := range routes {
		counts[route.RouteID] = route.PublishedCount
	}
	return counts
}

func waitForNoPendingExports(
	t *testing.T,
	archive *store.Store,
	timeout time.Duration,
) {
	t.Helper()
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		pending, err := archive.ListPendingMQTTExports(context.Background(), 100)
		if err != nil {
			t.Fatal(err)
		}
		if len(pending) == 0 {
			return
		}
		time.Sleep(100 * time.Millisecond)
	}
	pending, err := archive.ListPendingMQTTExports(context.Background(), 100)
	t.Fatalf("pending exports did not converge: %#v, %v", pending, err)
}

func readTestPassword(t *testing.T, envName string) string {
	t.Helper()
	path := requireEnv(t, envName)
	encoded, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	password := strings.TrimRight(string(encoded), "\r\n")
	if password == "" {
		t.Fatalf("%s is empty", envName)
	}
	return password
}

func writeTestMarker(t *testing.T, controlDir string, name string) {
	t.Helper()
	if err := os.WriteFile(filepath.Join(controlDir, name), []byte("ok\n"), 0o600); err != nil {
		t.Fatal(err)
	}
}

func waitForTestMarker(
	t *testing.T,
	controlDir string,
	name string,
	timeout time.Duration,
) {
	t.Helper()
	path := filepath.Join(controlDir, name)
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		if _, err := os.Stat(path); err == nil {
			return
		} else if !os.IsNotExist(err) {
			t.Fatal(err)
		}
		time.Sleep(100 * time.Millisecond)
	}
	t.Fatal(fmt.Errorf("timed out waiting for test marker %q", name))
}
