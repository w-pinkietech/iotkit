package siteapp

import (
	"context"
	"strings"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantic"
)

func TestProfileInputValidationMeasuresTrimmedTextAndRejectsControls(t *testing.T) {
	input := SignalProfileInput{DisplayName: strings.Repeat(" ", 200) + "温度"}
	if err := input.Validate(); err != nil {
		t.Fatalf("trimmed profile text was rejected: %v", err)
	}
	if err := (SignalProfileInput{DisplayName: "温度\n"}).Validate(); err == nil {
		t.Fatal("profile text containing a control character was accepted")
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
			Input:     SignalProfileInput{DisplayName: "乾燥炉入口温度"},
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
