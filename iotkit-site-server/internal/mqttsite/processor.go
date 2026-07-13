package mqttsite

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"

	"github.com/w-pinkietech/iotkit-next/iotkit-site-server/internal/contract"
)

type BatchStore interface {
	AcceptBatch(context.Context, contract.RecordBatch) (contract.AcceptedThrough, error)
}

type Publish func(topic string, payload []byte) error

type Processor struct {
	Store BatchStore
}

func (processor Processor) Process(ctx context.Context, topic string, payload []byte, publish Publish) error {
	if processor.Store == nil {
		return errors.New("MQTT processor store is nil")
	}
	if publish == nil {
		return errors.New("MQTT processor publish function is nil")
	}
	gatewayIdentity, err := recordsTopicGateway(topic)
	if err != nil {
		return err
	}
	batch, err := contract.DecodeBatch(payload)
	if err != nil {
		return err
	}
	if batch.GatewayIdentity != gatewayIdentity {
		return errors.New("MQTT topic/body gateway identity mismatch")
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
	ackTopic := "iotkit/v1/gateways/" + gatewayIdentity + "/accepted-through"
	return publish(ackTopic, ackPayload)
}

func recordsTopicGateway(topic string) (string, error) {
	parts := strings.Split(topic, "/")
	if len(parts) != 5 || parts[0] != "iotkit" || parts[1] != "v1" || parts[2] != "gateways" || parts[4] != "records" {
		return "", errors.New("unexpected MQTT records topic")
	}
	if parts[3] == "" || strings.ContainsAny(parts[3], "+#") {
		return "", errors.New("invalid MQTT records topic gateway identity")
	}
	return parts[3], nil
}
