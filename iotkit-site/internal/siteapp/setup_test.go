package siteapp

import (
	"context"
	"testing"
)

func TestSetupCandidateUsesClosedDescriptorMapping(t *testing.T) {
	tests := []struct {
		name           string
		measurementKey string
		valueType      string
		unit           *string
		wantSensorType string
		wantValueKind  string
		wantUnitMode   string
		wantUnit       string
		wantMissing    string
	}{
		{
			name:           "temperature descriptor",
			measurementKey: "temperature_c",
			valueType:      "float",
			unit:           stringPointer("Cel"),
			wantSensorType: "temperature",
			wantValueKind:  "numeric",
			wantUnitMode:   "unit",
			wantUnit:       "Cel",
		},
		{
			name:           "contact descriptor",
			measurementKey: "contact_state",
			valueType:      "bool",
			wantSensorType: "contact",
			wantValueKind:  "boolean",
			wantUnitMode:   "dimensionless",
		},
		{
			name:           "unknown descriptor",
			measurementKey: "vendor_magic",
			valueType:      "float",
			wantSensorType: "custom",
			wantValueKind:  "numeric",
			wantUnitMode:   "",
			wantMissing:    "display_sensor_type_label",
		},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			candidate, missing := setupCandidate(SignalSummary{
				SensorType: &test.measurementKey,
				ValueType:  &test.valueType,
				Unit:       test.unit,
			})
			if candidate.DisplaySensorType != test.wantSensorType ||
				candidate.DisplayValueKind != test.wantValueKind ||
				candidate.DisplayUnitMode != test.wantUnitMode ||
				candidate.DisplayUnit != test.wantUnit {
				t.Fatalf("candidate = %#v", candidate)
			}
			if test.wantMissing == "" {
				if len(missing) != 0 {
					t.Fatalf("missing = %#v, want none", missing)
				}
			} else if !containsString(missing, test.wantMissing) {
				t.Fatalf("missing = %#v, want %q", missing, test.wantMissing)
			}
		})
	}
}

func TestSetupStateIsDerivedFromProfilesAndMetadata(t *testing.T) {
	deviceRevision := int64(1)
	complete := SignalProfile{
		SignalRef:         "sig_00000000000000000000000000000001",
		DisplayName:       "温度",
		DisplaySensorType: "temperature",
		DisplayValueKind:  "numeric",
		DisplayUnitMode:   "unit",
		DisplayUnit:       "°C",
		DecimalPlaces:     1,
		Revision:          1,
	}
	tests := []struct {
		name    string
		device  DeviceSummary
		signal  SignalSummary
		profile *SignalProfile
		want    SetupState
	}{
		{
			name:   "device profile missing",
			device: DeviceSummary{},
			signal: SignalSummary{SensorType: stringPointer("temperature_c")},
			want:   SetupWaitingForDevice,
		},
		{
			name:   "signal profile missing",
			device: DeviceSummary{ProfileRevision: &deviceRevision},
			signal: SignalSummary{SensorType: stringPointer("temperature_c")},
			want:   SetupWaitingForSignal,
		},
		{
			name:   "metadata missing",
			device: DeviceSummary{ProfileRevision: &deviceRevision},
			signal: SignalSummary{},
			want:   SetupMetadataMissing,
		},
		{
			name:    "all profiles complete",
			device:  DeviceSummary{ProfileRevision: &deviceRevision},
			signal:  SignalSummary{SensorType: stringPointer("temperature_c")},
			profile: &complete,
			want:    SetupReady,
		},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			source := SetupDeviceSource{
				Device: test.device,
				Signals: []SetupSignalSource{{
					Signal:  test.signal,
					Profile: test.profile,
				}},
			}
			got := buildSetupDevice(source)
			if got.State != test.want {
				t.Fatalf("state = %q, want %q; setup=%#v", got.State, test.want, got)
			}
		})
	}
}

func TestListSetupDevicesHidesIdentifierFromViewer(t *testing.T) {
	identifier := "01234567"
	repository := &fakeRepository{
		setupDevices: []SetupDeviceSource{{
			Identifier: &identifier,
			Device: DeviceSummary{
				DeviceRef: "dev_00000000000000000000000000000001",
			},
		}},
	}
	service := NewService(repository)
	viewer, err := service.ListSetupDevices(
		context.Background(),
		AccountActor("acct_00000000000000000000000000000001", AccountRoleViewer),
		100,
	)
	if err != nil {
		t.Fatal(err)
	}
	if viewer[0].Identifier != nil {
		t.Fatalf("viewer received identifier: %#v", viewer[0].Identifier)
	}
	admin, err := service.ListSetupDevices(
		context.Background(),
		AccountActor("acct_00000000000000000000000000000002", AccountRoleAdmin),
		100,
	)
	if err != nil {
		t.Fatal(err)
	}
	if admin[0].Identifier == nil || *admin[0].Identifier != identifier {
		t.Fatalf("admin identifier = %#v", admin[0].Identifier)
	}
}

func stringPointer(value string) *string {
	return &value
}

func containsString(values []string, want string) bool {
	for _, value := range values {
		if value == want {
			return true
		}
	}
	return false
}
