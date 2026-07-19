package store

import (
	"fmt"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/outputadapter"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantics"
)

func outputObservation(
	observation semantics.Observation,
) (outputadapter.Observation, error) {
	kind, err := outputObservationKind(observation.Kind)
	if err != nil {
		return outputadapter.Observation{}, err
	}
	result := outputadapter.Observation{
		ObservationID: observation.ObservationID,
		SeriesID:      observation.SeriesID,
		Sequence:      observation.Sequence,
		ObservedAt:    observation.ObservedAt,
		Kind:          kind,
		Value:         observation.Value,
	}
	if err := result.Validate(); err != nil {
		return outputadapter.Observation{}, err
	}
	return result, nil
}

func outputObservationKind(
	kind semantics.Kind,
) (outputadapter.ObservationKind, error) {
	switch kind {
	case semantics.KindNumeric:
		return outputadapter.KindNumeric, nil
	case semantics.KindBoolean:
		return outputadapter.KindBoolean, nil
	case semantics.KindCumulativeCounter:
		return outputadapter.KindCumulativeValue, nil
	case semantics.KindAlarm:
		return outputadapter.KindAlarm, nil
	default:
		return "", fmt.Errorf(
			"%w: unsupported Site semantic kind %q",
			outputadapter.ErrInvalidObservation,
			kind,
		)
	}
}
