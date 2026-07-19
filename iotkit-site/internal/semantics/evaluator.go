package semantics

import (
	"errors"
	"math"
)

const maxSafeInteger = int64(9_007_199_254_740_991)

func Evaluate(spec DefinitionSpec, state State, input float64) (Result, State, error) {
	return EvaluateAt(spec, state, input, 0)
}

func EvaluateAt(
	spec DefinitionSpec,
	state State,
	input float64,
	receivedAt int64,
) (Result, State, error) {
	var noResult Result
	if err := spec.Validate(); err != nil {
		return noResult, state, err
	}
	if receivedAt < 0 {
		return noResult, state, errors.New("semantic received time must be non-negative")
	}
	if !finite(input) {
		return noResult, state, errors.New("semantic input must be finite")
	}
	calibrated := input*spec.Scale + spec.Offset
	if !finite(calibrated) {
		return noResult, state, errors.New("calibrated semantic input must be finite")
	}
	result := Result{Calibrated: calibrated}

	if spec.Kind == KindNumeric {
		value := calibrated
		result.Emitted = true
		result.Number = &value
		state.Initialized = true
		return result, state, nil
	}

	active, err := evaluateDetector(spec.Detector, state, calibrated)
	if err != nil {
		return noResult, state, err
	}
	wasInitialized := state.Initialized
	previousActive := state.Active
	if !wasInitialized {
		state.Initialized = true
		state.Active = active
		state.Pending = false
		return emitInitial(spec, result, state)
	}

	if active == state.Active {
		state.Pending = false
	} else {
		debounce := transitionDebounce(spec.Detector, active)
		switch {
		case debounce == 0:
			state.Active = active
			state.Pending = false
		case !state.Pending || state.PendingActive != active:
			state.Pending = true
			state.PendingActive = active
			state.PendingSince = receivedAt
			return result, state, nil
		case receivedAt < state.PendingSince:
			state.PendingSince = receivedAt
			return result, state, nil
		case receivedAt-state.PendingSince < debounce:
			return result, state, nil
		default:
			state.Active = active
			state.Pending = false
		}
	}
	state.Initialized = true
	active = state.Active

	switch spec.Kind {
	case KindBoolean, KindAlarm:
		if previousActive != active {
			value := active
			result.Emitted = true
			result.Boolean = &value
		}
	case KindCumulativeCounter:
		shouldIncrement := spec.Trigger == TriggerActiveSample && active ||
			spec.Trigger == TriggerTransition && !previousActive && active
		if !shouldIncrement {
			return result, state, nil
		}
		if state.Counter >= maxSafeInteger {
			return noResult, state, errors.New("cumulative counter reached the safe integer limit")
		}
		state.Counter++
		value := state.Counter
		result.Emitted = true
		result.Integer = &value
	default:
		return noResult, state, errors.New("unsupported semantic definition kind")
	}
	return result, state, nil
}

func emitInitial(spec DefinitionSpec, result Result, state State) (Result, State, error) {
	switch spec.Kind {
	case KindBoolean, KindAlarm:
		value := state.Active
		result.Emitted = true
		result.Boolean = &value
	case KindCumulativeCounter:
		// The first received value establishes the baseline and is never counted.
	default:
		return Result{}, state, errors.New("unsupported semantic definition kind")
	}
	return result, state, nil
}

func evaluateDetector(detector Detector, state State, value float64) (bool, error) {
	switch detector.Mode {
	case DetectorBooleanHighActive, DetectorBooleanLowActive:
		if value != 0 && value != 1 {
			return false, errors.New("boolean semantic input must be 0 or 1 after correction")
		}
		if detector.Mode == DetectorBooleanHighActive {
			return value == 1, nil
		}
		return value == 0, nil
	case DetectorHighActive:
		if state.Initialized && state.Active {
			return value > detector.FallThreshold, nil
		}
		return value >= detector.RiseThreshold, nil
	case DetectorLowActive:
		if state.Initialized && state.Active {
			return value < detector.RiseThreshold, nil
		}
		return value <= detector.FallThreshold, nil
	default:
		return false, errors.New("unsupported semantic detector")
	}
}

func transitionDebounce(detector Detector, targetActive bool) int64 {
	risingSignal := targetActive
	if detector.Mode == DetectorLowActive ||
		detector.Mode == DetectorBooleanLowActive {
		risingSignal = !targetActive
	}
	if risingSignal {
		return detector.RiseDebounceMS
	}
	return detector.FallDebounceMS
}

func IsSafeInteger(value float64) bool {
	return finite(value) && math.Trunc(value) == value &&
		value >= -float64(maxSafeInteger) && value <= float64(maxSafeInteger)
}
