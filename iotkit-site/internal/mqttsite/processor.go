package mqttsite

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/contract"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/store"
)

type BatchStore interface {
	AcceptBatch(context.Context, contract.RecordBatch) (contract.AcceptedThrough, error)
}

type DescriptorStore interface {
	ApplyDescriptorSnapshot(context.Context, contract.DescriptorSnapshot) (store.DescriptorApplyResult, error)
}

type MessageStore interface {
	BatchStore
	DescriptorStore
}

type Publish func(topic string, payload []byte) error

type Processor struct {
	Store MessageStore
}

func (processor Processor) Process(ctx context.Context, topic string, payload []byte, publish Publish) error {
	if processor.Store == nil {
		return errors.New("MQTT processor store is nil")
	}
	if edgeNodeID, err := descriptorsTopicEdgeNode(topic); err == nil {
		snapshot, err := contract.DecodeDescriptorSnapshot(payload)
		if err != nil {
			return err
		}
		if snapshot.EdgeNodeID != edgeNodeID {
			return errors.New("MQTT topic/body edge_node_id mismatch")
		}
		_, err = processor.Store.ApplyDescriptorSnapshot(ctx, snapshot)
		return err
	}
	edgeNodeID, err := recordsTopicEdgeNode(topic)
	if err != nil {
		return err
	}
	if publish == nil {
		return errors.New("MQTT processor publish function is nil")
	}
	batch, err := contract.DecodeBatch(payload)
	if err != nil {
		return err
	}
	if batch.EdgeNodeID != edgeNodeID {
		return errors.New("MQTT topic/body edge_node_id mismatch")
	}

	ack, err := processor.Store.AcceptBatch(ctx, batch)
	if err != nil {
		return err
	}
	if err := ack.ValidateFor(batch, batch.CursorStart-1); err != nil {
		return fmt.Errorf("store returned invalid accepted-through: %w", err)
	}
	ackPayload, err := json.Marshal(ack)
	if err != nil {
		return err
	}
	ackTopic := "iotkit/v1/edge-nodes/" + edgeNodeID + "/accepted-through"
	return publish(ackTopic, ackPayload)
}

func recordsTopicEdgeNode(topic string) (string, error) {
	return topicEdgeNode(topic, "records")
}

func descriptorsTopicEdgeNode(topic string) (string, error) {
	return topicEdgeNode(topic, "descriptors")
}

func topicEdgeNode(topic string, suffix string) (string, error) {
	parts := strings.Split(topic, "/")
	if len(parts) != 5 || parts[0] != "iotkit" || parts[1] != "v1" || parts[2] != "edge-nodes" || parts[4] != suffix {
		return "", fmt.Errorf("unexpected MQTT %s topic", suffix)
	}
	if parts[3] == "" || strings.ContainsAny(parts[3], "+#") {
		return "", fmt.Errorf("invalid MQTT %s topic edge node ID", suffix)
	}
	return parts[3], nil
}
