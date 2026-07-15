package siteapp

import (
	"context"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantic"
)

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
	mapping         semantic.Mapping
	applyCalls      int
	deactivateCalls int
	auditCalls      int
	routeCalls      int
	legacyRoute     LegacyMQTTRoute
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
