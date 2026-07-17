package outputadapter

import (
	"encoding/json"
	"errors"
	"fmt"
	"regexp"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantics"
)

type YokaKitKind string

const (
	YokaKitProduction YokaKitKind = "production"
	YokaKitOnOff      YokaKitKind = "onoff"
	YokaKitGanttChart YokaKitKind = "gantt_chart"
	YokaKitAlarm      YokaKitKind = "alarm"
)

var yokakitID = regexp.MustCompile(`^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$`)

type YokaKit struct {
	SourceID string
	SignalID string
	Kind     YokaKitKind
	Reason   string
}

func (adapter YokaKit) Validate() error {
	if !yokakitID.MatchString(adapter.SourceID) || !yokakitID.MatchString(adapter.SignalID) {
		return errors.New("YokaKit source_id and signal_id must use the closed topic ID syntax")
	}
	switch adapter.Kind {
	case YokaKitProduction, YokaKitOnOff, YokaKitGanttChart, YokaKitAlarm:
	default:
		return errors.New("unsupported YokaKit observation kind")
	}
	if len(adapter.Reason) > 512 {
		return errors.New("YokaKit alarm reason exceeds 512 bytes")
	}
	return nil
}

func (adapter YokaKit) Transform(observation semantics.Observation) (Message, error) {
	if err := adapter.Validate(); err != nil {
		return Message{}, err
	}
	if !compatibleYokaKitKind(observation.Kind, adapter.Kind) {
		return Message{}, ErrUnsupportedObservation
	}
	var value any
	if err := json.Unmarshal(observation.Value, &value); err != nil {
		return Message{}, fmt.Errorf("decode semantic observation value: %w", err)
	}
	payload := struct {
		SchemaVersion int             `json:"schema_version"`
		ObservationID string          `json:"observation_id"`
		SeriesID      string          `json:"series_id"`
		Sequence      int64           `json:"sequence"`
		ObservedAt    int64           `json:"observed_at"`
		Kind          YokaKitKind     `json:"kind"`
		Value         any             `json:"value"`
		Reason        string          `json:"reason,omitempty"`
		Reading       json.RawMessage `json:"reading,omitempty"`
	}{
		SchemaVersion: 1,
		ObservationID: observation.ObservationID,
		SeriesID:      observation.SeriesID,
		Sequence:      observation.Sequence,
		ObservedAt:    observation.ObservedAt,
		Kind:          adapter.Kind,
		Value:         value,
	}
	if adapter.Kind == YokaKitAlarm {
		payload.Reason = adapter.Reason
	}
	encoded, err := json.Marshal(payload)
	if err != nil {
		return Message{}, err
	}
	return Message{
		Topic: "yokakit/v1/sources/" + adapter.SourceID + "/signals/" +
			adapter.SignalID + "/observations",
		QoS:     1,
		Retain:  false,
		Payload: encoded,
	}, nil
}

func compatibleYokaKitKind(source semantics.Kind, target YokaKitKind) bool {
	switch target {
	case YokaKitProduction:
		return source == semantics.KindCumulativeCounter
	case YokaKitOnOff, YokaKitGanttChart:
		return source == semantics.KindBoolean
	case YokaKitAlarm:
		return source == semantics.KindAlarm
	default:
		return false
	}
}

func YokaKitStatus(sourceID string, reportedAt int64) (Message, error) {
	if !yokakitID.MatchString(sourceID) || reportedAt < 0 ||
		reportedAt > 253_402_300_799_999 {
		return Message{}, errors.New("invalid YokaKit source status")
	}
	payload, err := json.Marshal(struct {
		SchemaVersion int    `json:"schema_version"`
		ReportedAt    int64  `json:"reported_at"`
		State         string `json:"state"`
	}{1, reportedAt, "online"})
	if err != nil {
		return Message{}, err
	}
	return Message{
		Topic:   "yokakit/v1/sources/" + sourceID + "/status",
		QoS:     1,
		Retain:  true,
		Payload: payload,
	}, nil
}
