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

func TestMQTTRetainedDescriptorIsAvailableToLateSubscriber(t *testing.T) {
	brokerURL := requireEnv(t, "IOTKIT_TEST_BROKER_URL")
	passwordPath := requireEnv(t, "IOTKIT_TEST_SITE_PASSWORD_FILE")
	edgeNodeID := requireEnv(t, "IOTKIT_TEST_EDGE_NODE_ID")
	passwordBytes, err := os.ReadFile(passwordPath)
	if err != nil {
		t.Fatal(err)
	}
	options := mqtt.NewClientOptions().
		AddBroker(brokerURL).
		SetClientID("iotkit-site-descriptor-late-subscriber").
		SetUsername("site").
		SetPassword(strings.TrimRight(string(passwordBytes), "\r\n"))
	client := mqtt.NewClient(options)
	if token := client.Connect(); !token.WaitTimeout(10 * time.Second) {
		t.Fatal("MQTT connect timeout")
	} else if err := token.Error(); err != nil {
		t.Fatal(err)
	}
	defer client.Disconnect(250)

	messages := make(chan mqtt.Message, 1)
	topic := "iotkit/v1/edge-nodes/" + edgeNodeID + "/descriptors"
	if token := client.Subscribe(topic, 1, func(_ mqtt.Client, message mqtt.Message) {
		messages <- message
	}); !token.WaitTimeout(10 * time.Second) {
		t.Fatal("MQTT subscribe timeout")
	} else if err := token.Error(); err != nil {
		t.Fatal(err)
	}
	select {
	case message := <-messages:
		if !message.Retained() {
			t.Fatal("late subscriber received a non-retained descriptor")
		}
		snapshot, err := contract.DecodeDescriptorSnapshot(message.Payload())
		if err != nil {
			t.Fatal(err)
		}
		if snapshot.EdgeNodeID != edgeNodeID || !snapshot.Complete {
			t.Fatalf("snapshot = %#v", snapshot)
		}
	case <-time.After(15 * time.Second):
		t.Fatal("retained descriptor timeout")
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
