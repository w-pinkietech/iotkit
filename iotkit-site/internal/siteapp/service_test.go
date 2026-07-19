package siteapp

import (
	"context"
	"errors"
	"strings"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantic"
)

func TestProfileInputValidationMeasuresTrimmedTextAndRejectsControls(t *testing.T) {
	input := validSignalProfileInput()
	input.DisplayName = strings.Repeat(" ", 200) + "温度"
	if err := input.Validate(); err != nil {
		t.Fatalf("trimmed profile text was rejected: %v", err)
	}
	input = validSignalProfileInput()
	input.DisplayName = "温度\n"
	if err := input.Validate(); err == nil {
		t.Fatal("profile text containing a control character was accepted")
	}
}

func TestActivateEdgeRequiresAnAdministrator(t *testing.T) {
	repository := &fakeRepository{}
	service := NewService(repository)
	expected := int64(1)

	_, err := service.Dispatch(
		context.Background(),
		AccountActor(
			"acct_00000000000000000000000000000001",
			AccountRoleViewer,
		),
		ActivateEdge{
			EdgeRef: "edge_00000000000000000000000000000001",
			Precondition: RevisionPrecondition{
				Expected: &expected,
			},
		},
	)
	if !errors.Is(err, ErrForbidden) {
		t.Fatalf("Dispatch error = %v, want ErrForbidden", err)
	}
	if repository.activateEdgeCalls != 0 {
		t.Fatalf("activation calls = %d, want 0", repository.activateEdgeCalls)
	}
}

func TestAdministratorCanActivateAndListEdges(t *testing.T) {
	edge := Edge{
		EdgeRef:     "edge_00000000000000000000000000000001",
		EdgeNodeID:  "factory-edge-01",
		LedgerEpoch: "epoch-01",
		State:       EdgeActivating,
		Revision:    2,
	}
	repository := &fakeRepository{edges: []Edge{edge}, edge: edge}
	service := NewService(repository)
	expected := int64(1)

	result, err := service.Dispatch(
		context.Background(),
		AccountActor(
			"acct_00000000000000000000000000000001",
			AccountRoleAdmin,
		),
		ActivateEdge{
			EdgeRef: edge.EdgeRef,
			Precondition: RevisionPrecondition{
				Expected: &expected,
			},
		},
	)
	if err != nil {
		t.Fatal(err)
	}
	if result.Edge == nil || result.Edge.EdgeRef != edge.EdgeRef {
		t.Fatalf("result = %#v", result)
	}
	edges, err := service.ListEdges(context.Background())
	if err != nil || len(edges) != 1 || edges[0].EdgeRef != edge.EdgeRef {
		t.Fatalf("edges = %#v, err = %v", edges, err)
	}
}

func TestSignalProfileV2Validation(t *testing.T) {
	tests := []struct {
		name    string
		mutate  func(*SignalProfileInput)
		wantErr bool
	}{
		{name: "thermocouple numeric with unit"},
		{
			name: "contact boolean without unit",
			mutate: func(input *SignalProfileInput) {
				input.DisplaySensorType = "contact"
				input.DisplayValueKind = "boolean"
				input.DisplayUnitMode = "dimensionless"
				input.DisplayUnit = ""
				input.DecimalPlaces = 0
			},
		},
		{
			name: "unknown sensor type",
			mutate: func(input *SignalProfileInput) {
				input.DisplaySensorType = "vendor_magic"
			},
			wantErr: true,
		},
		{
			name: "custom without label",
			mutate: func(input *SignalProfileInput) {
				input.DisplaySensorType = "custom"
				input.DisplaySensorTypeLabel = ""
			},
			wantErr: true,
		},
		{
			name: "unknown value kind",
			mutate: func(input *SignalProfileInput) {
				input.DisplayValueKind = "record"
			},
			wantErr: true,
		},
		{
			name: "boolean with unit",
			mutate: func(input *SignalProfileInput) {
				input.DisplayValueKind = "boolean"
				input.DisplayUnitMode = "unit"
			},
			wantErr: true,
		},
		{
			name: "numeric unit mode without unit",
			mutate: func(input *SignalProfileInput) {
				input.DisplayUnit = ""
			},
			wantErr: true,
		},
		{
			name: "negative decimal places",
			mutate: func(input *SignalProfileInput) {
				input.DecimalPlaces = -1
			},
			wantErr: true,
		},
		{
			name: "too many decimal places",
			mutate: func(input *SignalProfileInput) {
				input.DecimalPlaces = 7
			},
			wantErr: true,
		},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			input := validSignalProfileInput()
			if test.mutate != nil {
				test.mutate(&input)
			}
			err := input.Validate()
			if test.wantErr && err == nil {
				t.Fatal("invalid signal profile was accepted")
			}
			if !test.wantErr && err != nil {
				t.Fatalf("valid signal profile was rejected: %v", err)
			}
		})
	}
}

func TestDispatchValidatesActorBeforeRepositoryMutation(t *testing.T) {
	repository := &fakeRepository{}
	service := NewService(repository)
	_, err := service.Dispatch(context.Background(), Actor{}, PutSemanticMapping{Spec: validSpec()})
	if err == nil {
		t.Fatal("empty actor was accepted")
	}
	if repository.applyCalls != 0 {
		t.Fatalf("repository apply calls = %d", repository.applyCalls)
	}
}

func TestDispatchRoutesPutAndDeactivateOperations(t *testing.T) {
	repository := &fakeRepository{mapping: semantic.Mapping{
		ID:       "sm-01",
		Revision: 1,
		MappingSpec: semantic.MappingSpec{
			EdgeNodeID:  "edge-node-01",
			SeriesKey:   "series-01",
			Meaning:     semantic.MeaningProductionPulse,
			TriggerMode: semantic.TriggerActiveEdge,
			ActiveValue: 1,
		},
		Active: true,
	}}
	service := NewService(repository)
	put, err := service.Dispatch(context.Background(), LocalCLIActor(), PutSemanticMapping{Spec: validSpec()})
	if err != nil || put.SemanticMapping == nil {
		t.Fatalf("put result = %#v, err = %v", put, err)
	}
	deactivate, err := service.Dispatch(context.Background(), LocalCLIActor(), DeactivateSemanticMapping{
		EdgeNodeID: "edge-node-01",
		SeriesKey:  "series-01",
	})
	if err != nil || deactivate.SemanticMapping == nil {
		t.Fatalf("deactivate result = %#v, err = %v", deactivate, err)
	}
	if repository.applyCalls != 1 || repository.deactivateCalls != 1 {
		t.Fatalf("repository calls = apply %d, deactivate %d", repository.applyCalls, repository.deactivateCalls)
	}
}

func TestDispatchRoutesLegacyMQTTRouteOperation(t *testing.T) {
	repository := &fakeRepository{legacyRoute: LegacyMQTTRoute{
		RouteID:   "mr-01",
		MappingID: "sm-01",
		Topic:     "factory/production-pulses",
		QoS:       1,
	}}
	service := NewService(repository)
	result, err := service.Dispatch(context.Background(), LocalCLIActor(), PutLegacyMQTTRoute{
		MappingID: "sm-01",
		Topic:     "factory/production-pulses",
	})
	if err != nil || result.LegacyMQTTRoute == nil {
		t.Fatalf("route result = %#v, err = %v", result, err)
	}
	if repository.routeCalls != 1 {
		t.Fatalf("repository route calls = %d", repository.routeCalls)
	}
}

func TestDispatchRoutesInventoryProfileOperations(t *testing.T) {
	deviceRef := "dev_00000000000000000000000000000001"
	signalRef := "sig_00000000000000000000000000000001"
	repository := &fakeRepository{
		deviceProfile: DeviceProfile{DeviceRef: deviceRef, Revision: 1},
		signalProfile: SignalProfile{SignalRef: signalRef, Revision: 1},
	}
	service := NewService(repository)
	deviceResult, err := service.Dispatch(
		context.Background(),
		LocalCLIActor(),
		UpdateDeviceProfile{
			DeviceRef: deviceRef,
			Input: DeviceProfileInput{
				DisplayName: "乾燥炉入口",
				Location:    "第2工場",
			},
		},
	)
	if err != nil || deviceResult.DeviceProfile == nil {
		t.Fatalf("device profile result = %#v, err = %v", deviceResult, err)
	}
	signalResult, err := service.Dispatch(
		context.Background(),
		LocalCLIActor(),
		UpdateSignalProfile{
			SignalRef: signalRef,
			Input:     validSignalProfileInput(),
		},
	)
	if err != nil || signalResult.SignalProfile == nil {
		t.Fatalf("signal profile result = %#v, err = %v", signalResult, err)
	}
	if repository.deviceProfileCalls != 1 || repository.signalProfileCalls != 1 {
		t.Fatalf("profile calls = device %d, signal %d",
			repository.deviceProfileCalls, repository.signalProfileCalls)
	}
}

func validSignalProfileInput() SignalProfileInput {
	return SignalProfileInput{
		DisplayName:       "乾燥炉入口熱電対",
		DisplaySensorType: "thermocouple",
		DisplayValueKind:  "numeric",
		DisplayUnitMode:   "unit",
		DisplayUnit:       "°C",
		DecimalPlaces:     1,
	}
}

func TestDispatchRejectsInvalidProfileBeforeRepositoryMutation(t *testing.T) {
	repository := &fakeRepository{}
	service := NewService(repository)
	_, err := service.Dispatch(
		context.Background(),
		LocalCLIActor(),
		UpdateDeviceProfile{
			DeviceRef: "dev_00000000000000000000000000000001",
			Input:     DeviceProfileInput{DisplayName: "", Location: "第2工場"},
		},
	)
	if err == nil {
		t.Fatal("invalid profile was accepted")
	}
	if repository.deviceProfileCalls != 0 {
		t.Fatalf("device profile calls = %d", repository.deviceProfileCalls)
	}
}

func TestDispatchRejectsInvalidMappingBeforeRepositoryMutation(t *testing.T) {
	repository := &fakeRepository{}
	service := NewService(repository)
	_, err := service.Dispatch(context.Background(), LocalCLIActor(), PutSemanticMapping{Spec: semantic.MappingSpec{}})
	if err == nil {
		t.Fatal("invalid mapping was accepted")
	}
	if repository.applyCalls != 0 {
		t.Fatalf("repository apply calls = %d", repository.applyCalls)
	}
}

func TestListAuditEventsRejectsUnboundedLimit(t *testing.T) {
	repository := &fakeRepository{}
	service := NewService(repository)
	if _, err := service.ListAuditEvents(context.Background(), 101); err == nil {
		t.Fatal("limit 101 was accepted")
	}
	if repository.auditCalls != 0 {
		t.Fatalf("repository audit calls = %d", repository.auditCalls)
	}
}

func TestListInventoryValidatesPageBeforeRepositoryQuery(t *testing.T) {
	repository := &fakeRepository{}
	service := NewService(repository)
	if _, err := service.ListDevices(context.Background(), PageRequest{Limit: 101}); err == nil {
		t.Fatal("device page limit 101 was accepted")
	}
	if _, err := service.ListSignals(context.Background(), PageRequest{Limit: 0}); err == nil {
		t.Fatal("signal page limit 0 was accepted")
	}
	if repository.deviceListCalls != 0 || repository.signalListCalls != 0 {
		t.Fatalf("inventory repository calls = device %d, signal %d",
			repository.deviceListCalls, repository.signalListCalls)
	}
}

func TestListInventoryDelegatesBoundedPage(t *testing.T) {
	repository := &fakeRepository{
		devices: []DeviceSummary{{DeviceRef: "dev_00000000000000000000000000000001"}},
		signals: []SignalSummary{{SignalRef: "sig_00000000000000000000000000000001"}},
	}
	service := NewService(repository)
	devices, err := service.ListDevices(context.Background(), PageRequest{Limit: 10})
	if err != nil || len(devices) != 1 {
		t.Fatalf("devices = %#v, err = %v", devices, err)
	}
	signals, err := service.ListSignals(context.Background(), PageRequest{Limit: 10})
	if err != nil || len(signals) != 1 {
		t.Fatalf("signals = %#v, err = %v", signals, err)
	}
}

func validSpec() semantic.MappingSpec {
	return semantic.MappingSpec{
		EdgeNodeID:  "edge-node-01",
		SeriesKey:   "series-01",
		Meaning:     semantic.MeaningProductionPulse,
		TriggerMode: semantic.TriggerActiveEdge,
		ActiveValue: 1,
	}
}

type fakeRepository struct {
	mapping            semantic.Mapping
	applyCalls         int
	deactivateCalls    int
	auditCalls         int
	routeCalls         int
	legacyRoute        LegacyMQTTRoute
	deviceProfile      DeviceProfile
	signalProfile      SignalProfile
	deviceProfileCalls int
	signalProfileCalls int
	devices            []DeviceSummary
	signals            []SignalSummary
	setupDevices       []SetupDeviceSource
	deviceListCalls    int
	signalListCalls    int
	edges              []Edge
	edge               Edge
	activateEdgeCalls  int
}

func (repository *fakeRepository) ListEdges(context.Context) ([]Edge, error) {
	return repository.edges, nil
}

func (repository *fakeRepository) RequestEdgeActivation(
	context.Context,
	Actor,
	string,
	RevisionPrecondition,
) (Edge, error) {
	repository.activateEdgeCalls++
	return repository.edge, nil
}

func (repository *fakeRepository) ListSetupDevices(
	context.Context,
	int,
) ([]SetupDeviceSource, error) {
	return repository.setupDevices, nil
}

func (repository *fakeRepository) ListInventoryDevices(
	context.Context,
	int,
	string,
) ([]DeviceSummary, error) {
	repository.deviceListCalls++
	return repository.devices, nil
}

func (repository *fakeRepository) ListInventorySignals(
	context.Context,
	int,
	string,
) ([]SignalSummary, error) {
	repository.signalListCalls++
	return repository.signals, nil
}

func (repository *fakeRepository) UpdateDeviceProfile(
	context.Context,
	Actor,
	string,
	DeviceProfileInput,
	RevisionPrecondition,
) (DeviceProfile, error) {
	repository.deviceProfileCalls++
	return repository.deviceProfile, nil
}

func (repository *fakeRepository) UpdateSignalProfile(
	context.Context,
	Actor,
	string,
	SignalProfileInput,
	RevisionPrecondition,
) (SignalProfile, error) {
	repository.signalProfileCalls++
	return repository.signalProfile, nil
}

func (repository *fakeRepository) ApplySemanticMapping(
	context.Context,
	Actor,
	semantic.MappingSpec,
	RevisionPrecondition,
) (semantic.Mapping, error) {
	repository.applyCalls++
	return repository.mapping, nil
}

func (repository *fakeRepository) DeactivateSemanticMapping(
	context.Context,
	Actor,
	string,
	string,
	RevisionPrecondition,
) (semantic.Mapping, error) {
	repository.deactivateCalls++
	mapping := repository.mapping
	mapping.Active = false
	return mapping, nil
}

func (repository *fakeRepository) ListSemanticMappings(context.Context) ([]semantic.Mapping, error) {
	return []semantic.Mapping{repository.mapping}, nil
}

func (repository *fakeRepository) ListAuditEvents(context.Context, int) ([]AuditEvent, error) {
	repository.auditCalls++
	return nil, nil
}

func (repository *fakeRepository) ApplyLegacyMQTTRoute(
	context.Context,
	Actor,
	string,
	string,
) (LegacyMQTTRoute, error) {
	repository.routeCalls++
	return repository.legacyRoute, nil
}
