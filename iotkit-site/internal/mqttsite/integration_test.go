//go:build integration

package mqttsite

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	mqtt "github.com/eclipse/paho.mqtt.golang"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/contract"
)

func TestMQTTFixtureGetsApplicationAcknowledgement(t *testing.T) {
	brokerURL := requireEnv(t, "IOTKIT_TEST_BROKER_URL")
	passwordPath := requireEnv(t, "IOTKIT_TEST_EDGE_PASSWORD_FILE")
	passwordBytes, err := os.ReadFile(passwordPath)
	if err != nil {
		t.Fatal(err)
	}
	batchPath := filepath.Join("..", "..", "..", "testdata", "egress", "v1", "record-batch.json")
	batchPayload, err := os.ReadFile(batchPath)
	if err != nil {
		t.Fatal(err)
	}

	options := mqtt.NewClientOptions().
		AddBroker(brokerURL).
		SetClientID("iotkit-edge-integration-test").
		SetUsername("edge-node-01").
		SetPassword(strings.TrimRight(string(passwordBytes), "\r\n"))
	client := mqtt.NewClient(options)
	if token := client.Connect(); !token.WaitTimeout(10 * time.Second) {
		t.Fatal("MQTT connect timeout")
	} else if err := token.Error(); err != nil {
		t.Fatal(err)
	}
	defer client.Disconnect(250)

	acks := make(chan []byte, 1)
	ackTopic := "iotkit/v1/edge-nodes/edge-node-01/accepted-through"
	if token := client.Subscribe(ackTopic, 1, func(_ mqtt.Client, message mqtt.Message) {
		acks <- append([]byte(nil), message.Payload()...)
	}); !token.WaitTimeout(10 * time.Second) {
		t.Fatal("MQTT subscribe timeout")
	} else if err := token.Error(); err != nil {
		t.Fatal(err)
	}

	recordsTopic := "iotkit/v1/edge-nodes/edge-node-01/records"
	publish := func() {
		t.Helper()
		if token := client.Publish(recordsTopic, 1, false, batchPayload); !token.WaitTimeout(5 * time.Second) {
			t.Fatal("MQTT publish timeout")
		} else if err := token.Error(); err != nil {
			t.Fatal(err)
		}
	}
	publish()
	retry := time.NewTicker(500 * time.Millisecond)
	defer retry.Stop()
	deadline := time.NewTimer(15 * time.Second)
	defer deadline.Stop()
	for {
		select {
		case payload := <-acks:
			ack, err := contract.DecodeAcceptedThrough(payload)
			if err != nil {
				t.Fatal(err)
			}
			if ack.EdgeNodeID != "edge-node-01" || ack.AcceptedThrough != 1 {
				t.Fatalf("unexpected accepted-through: %+v", ack)
			}
			return
		case <-retry.C:
			publish()
		case <-deadline.C:
			t.Fatal("application accepted-through timeout")
		}
	}
}

func requireEnv(t *testing.T, name string) string {
	t.Helper()
	value := os.Getenv(name)
	if value == "" {
		t.Fatalf("%s is required", name)
	}
	return value
}
