package outputadapter

import (
	"encoding/json"
	"errors"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantics"
)

type Message struct {
	Topic   string
	QoS     byte
	Retain  bool
	Payload json.RawMessage
}

type Adapter interface {
	Transform(semantics.Observation) (Message, error)
}

var ErrUnsupportedObservation = errors.New("output adapter does not support the observation")
