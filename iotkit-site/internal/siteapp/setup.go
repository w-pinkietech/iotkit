package siteapp

import (
	"context"
	"errors"
	"strings"
)

type SetupState string

const (
	SetupWaitingForDevice SetupState = "waiting_for_device"
	SetupWaitingForSignal SetupState = "waiting_for_signal"
	SetupMetadataMissing  SetupState = "metadata_missing"
	SetupReady            SetupState = "ready"
)

type SetupSignal struct {
	Signal           SignalSummary      `json:"signal"`
	ChannelIndex     *int32             `json:"channel_index,omitempty"`
	Profile          *SignalProfile     `json:"profile,omitempty"`
	ProfileComplete  bool               `json:"profile_complete"`
	Candidate        SignalProfileInput `json:"candidate"`
	CandidateMissing []string           `json:"candidate_missing"`
}

type SetupDevice struct {
	Device     DeviceSummary `json:"device"`
	Identifier *string       `json:"identifier,omitempty"`
	State      SetupState    `json:"setup_state"`
	Signals    []SetupSignal `json:"signals"`
}

type descriptorCandidate struct {
	sensorType string
	valueKind  string
	unit       string
}

var descriptorCandidates = map[string]descriptorCandidate{
	"temperature_c":             {sensorType: "thermocouple", valueKind: "numeric", unit: "°C"},
	"contact_state":             {sensorType: "contact", valueKind: "boolean"},
	"illuminance_lux":           {sensorType: "illuminance", valueKind: "numeric", unit: "lx"},
	"distance_mm":               {sensorType: "distance", valueKind: "numeric", unit: "mm"},
	"voltage_mv":                {sensorType: "voltage", valueKind: "numeric", unit: "mV"},
	"current_ma":                {sensorType: "current", valueKind: "numeric", unit: "mA"},
	"differential_pressure_pa":  {sensorType: "pressure", valueKind: "numeric", unit: "Pa"},
	"relative_humidity_percent": {sensorType: "humidity", valueKind: "numeric", unit: "%RH"},
	"acceleration_mg":           {sensorType: "acceleration", valueKind: "numeric", unit: "mg"},
}

func (service *Service) ListSetupDevices(
	ctx context.Context,
	actor Actor,
	limit int,
) ([]SetupDevice, error) {
	if err := actor.Validate(); err != nil {
		return nil, err
	}
	if limit < 1 || limit > 100 {
		return nil, errors.New("setup device limit must be between 1 and 100")
	}
	sources, err := service.repository.ListSetupDevices(ctx, limit)
	if err != nil {
		return nil, err
	}
	result := make([]SetupDevice, 0, len(sources))
	for _, source := range sources {
		device := buildSetupDevice(source)
		if actor.Class == ActorAccount && actor.Role == AccountRoleViewer {
			device.Identifier = nil
			device.Device.Identifier = nil
		}
		result = append(result, device)
	}
	return result, nil
}

func buildSetupDevice(source SetupDeviceSource) SetupDevice {
	device := SetupDevice{
		Device:     source.Device,
		Identifier: source.Identifier,
		State:      SetupReady,
		Signals:    make([]SetupSignal, 0, len(source.Signals)),
	}
	if source.Device.ProfileRevision == nil {
		device.State = SetupWaitingForDevice
	}
	for _, sourceSignal := range source.Signals {
		candidate, missing := setupCandidate(sourceSignal.Signal)
		complete := sourceSignal.Profile != nil && sourceSignal.Profile.Complete()
		device.Signals = append(device.Signals, SetupSignal{
			Signal:           sourceSignal.Signal,
			ChannelIndex:     sourceSignal.ChannelIndex,
			Profile:          sourceSignal.Profile,
			ProfileComplete:  complete,
			Candidate:        candidate,
			CandidateMissing: missing,
		})
		if source.Device.ProfileRevision == nil || complete {
			continue
		}
		if len(missing) > 0 {
			device.State = SetupMetadataMissing
		} else if device.State != SetupMetadataMissing {
			device.State = SetupWaitingForSignal
		}
	}
	return device
}

func setupCandidate(signal SignalSummary) (SignalProfileInput, []string) {
	candidate := SignalProfileInput{DecimalPlaces: 0}
	missing := make([]string, 0, 3)
	measurementKey := pointerValue(signal.SensorType)
	if known, exists := descriptorCandidates[measurementKey]; exists {
		candidate.DisplaySensorType = known.sensorType
		candidate.DisplayValueKind = known.valueKind
		if known.valueKind == "boolean" {
			candidate.DisplayUnitMode = "dimensionless"
		} else {
			candidate.DisplayUnitMode = "unit"
			candidate.DisplayUnit = known.unit
			candidate.DecimalPlaces = 1
		}
	} else if measurementKey != "" {
		candidate.DisplaySensorType = "custom"
		missing = append(missing, "display_sensor_type_label")
	}

	switch pointerValue(signal.ValueType) {
	case "bool":
		candidate.DisplayValueKind = "boolean"
		candidate.DisplayUnitMode = "dimensionless"
		candidate.DisplayUnit = ""
		candidate.DecimalPlaces = 0
	case "float", "int":
		candidate.DisplayValueKind = "numeric"
	default:
		if candidate.DisplayValueKind == "" {
			missing = append(missing, "display_value_kind")
		}
	}
	if unit := strings.TrimSpace(pointerValue(signal.Unit)); unit != "" &&
		candidate.DisplayValueKind != "boolean" {
		candidate.DisplayUnitMode = "unit"
		candidate.DisplayUnit = unit
	}
	if candidate.DisplaySensorType == "" {
		missing = append(missing, "display_sensor_type")
	}
	if candidate.DisplayValueKind == "numeric" && candidate.DisplayUnitMode == "" {
		missing = append(missing, "display_unit_mode")
	}
	return candidate, missing
}

func SignalProfileCandidate(signal SignalSummary) (SignalProfileInput, []string) {
	return setupCandidate(signal)
}

func pointerValue(value *string) string {
	if value == nil {
		return ""
	}
	return *value
}
