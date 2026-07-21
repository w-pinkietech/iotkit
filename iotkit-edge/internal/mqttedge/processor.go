package mqttedge

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/contract"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/store"
)

type BatchStore interface {
	AcceptBatch(context.Context, contract.RecordBatch) (contract.AcceptedThrough, error)
}

type DescriptorStore interface {
	ApplyDescriptorSnapshot(context.Context, contract.DescriptorSnapshot) (store.DescriptorApplyResult, error)
}

type ActivationStore interface {
	ApplyActivationResult(context.Context, contract.ActivationResult) (store.EdgeNodeActivation, error)
}

type MessageStore interface {
	BatchStore
	DescriptorStore
	ActivationStore
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
	if edgeNodeID, err := activationResultTopicEdgeNode(topic); err == nil {
		result, err := contract.DecodeActivationResult(payload)
		if err != nil {
			return err
		}
		if err := result.ValidateTopicEdgeNode(edgeNodeID); err != nil {
			return err
		}
		_, err = processor.Store.ApplyActivationResult(ctx, result)
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

func activationResultTopicEdgeNode(topic string) (string, error) {
	parts := strings.Split(topic, "/")
	if len(parts) != 6 ||
		parts[0] != "iotkit" ||
		parts[1] != "v1" ||
		parts[2] != "edge-nodes" ||
		parts[4] != "activation" ||
		parts[5] != "result" {
		return "", errors.New("unexpected MQTT activation result topic")
	}
	if parts[3] == "" || strings.ContainsAny(parts[3], "+#") {
		return "", errors.New("invalid MQTT activation result topic Edge Node ID")
	}
	return parts[3], nil
}

func topicEdgeNode(topic string, suffix string) (string, error) {
	parts := strings.Split(topic, "/")
	if len(parts) != 5 || parts[0] != "iotkit" || parts[1] != "v1" || parts[2] != "edge-nodes" || parts[4] != suffix {
		return "", fmt.Errorf("unexpected MQTT %s topic", suffix)
	}
	if parts[3] == "" || strings.ContainsAny(parts[3], "+#") {
		return "", fmt.Errorf("invalid MQTT %s topic Edge Node ID", suffix)
	}
	return parts[3], nil
}
