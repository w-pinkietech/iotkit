package semantics

import (
	"errors"
	"math"
)

const maxSafeInteger = int64(9_007_199_254_740_991)

func Evaluate(spec DefinitionSpec, state State, input float64) (Result, State, error) {
	var noResult Result
	if err := spec.Validate(); err != nil {
		return noResult, state, err
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

	active, err := evaluateCondition(spec.Condition, state, calibrated)
	if err != nil {
		return noResult, state, err
	}
	wasInitialized := state.Initialized
	previousActive := state.Active
	state.Initialized = true
	state.Active = active

	switch spec.Kind {
	case KindBoolean, KindAlarm:
		if !wasInitialized || previousActive != active {
			value := active
			result.Emitted = true
			result.Boolean = &value
		}
	case KindCumulativeCounter:
		if !wasInitialized {
			return result, state, nil
		}
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

func evaluateCondition(condition Condition, state State, value float64) (bool, error) {
	switch condition.Mode {
	case ConditionBoolean:
		if value != 0 && value != 1 {
			return false, errors.New("boolean semantic input must be 0 or 1 after correction")
		}
		return (value == 1) == condition.BoolValue, nil
	case ConditionAbove:
		if state.Initialized && state.Active {
			return value > condition.Threshold-condition.Hysteresis, nil
		}
		return value >= condition.Threshold, nil
	case ConditionBelow:
		if state.Initialized && state.Active {
			return value < condition.Threshold+condition.Hysteresis, nil
		}
		return value <= condition.Threshold, nil
	default:
		return false, errors.New("unsupported semantic condition")
	}
}

func IsSafeInteger(value float64) bool {
	return finite(value) && math.Trunc(value) == value &&
		value >= -float64(maxSafeInteger) && value <= float64(maxSafeInteger)
}
