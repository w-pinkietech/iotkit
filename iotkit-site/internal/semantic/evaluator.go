package semantic

import "errors"

func Evaluate(mode TriggerMode, activeValue int, previous *int, current int) (emit bool, next int, err error) {
	if current != 0 && current != 1 {
		return false, 0, errors.New("contact value must be 0 or 1")
	}

	switch mode {
	case TriggerActiveSample:
		return current == activeValue, current, nil
	case TriggerActiveEdge:
		return previous != nil && *previous != activeValue && current == activeValue, current, nil
	default:
		return false, 0, errors.New("unsupported trigger mode")
	}
}
