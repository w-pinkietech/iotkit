package siteapp

import (
	"context"
	"errors"
	"strings"
	"unicode"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantics"
)

type SemanticConfigurationRepository interface {
	GetSemanticConfiguration(context.Context, string) (semantics.Configuration, error)
	UpdateSignalCalibration(
		context.Context,
		Actor,
		string,
		float64,
		float64,
		RevisionPrecondition,
	) (semantics.Configuration, error)
	CreateSemanticRule(
		context.Context,
		Actor,
		string,
		string,
		semantics.RuleSpec,
		RevisionPrecondition,
	) (semantics.Rule, error)
	UpdateSemanticRule(
		context.Context,
		Actor,
		string,
		string,
		semantics.RuleSpec,
		RevisionPrecondition,
	) (semantics.Rule, error)
	RetireSemanticRule(
		context.Context,
		Actor,
		string,
		RevisionPrecondition,
	) (semantics.Rule, error)
	RequestSemanticCounterReset(
		context.Context,
		Actor,
		string,
		string,
	) (semantics.CounterReset, error)
}

type SemanticConfigurationService struct {
	repository SemanticConfigurationRepository
}

func NewSemanticConfigurationService(
	repository SemanticConfigurationRepository,
) *SemanticConfigurationService {
	return &SemanticConfigurationService{repository: repository}
}

func (service *SemanticConfigurationService) Get(
	ctx context.Context,
	actor Actor,
	signalRef string,
) (semantics.Configuration, error) {
	if err := service.ready(actor); err != nil {
		return semantics.Configuration{}, err
	}
	if err := validateResourceRef(signalRef, "sig_"); err != nil {
		return semantics.Configuration{}, err
	}
	return service.repository.GetSemanticConfiguration(ctx, signalRef)
}

func (service *SemanticConfigurationService) UpdateCalibration(
	ctx context.Context,
	actor Actor,
	signalRef string,
	scale float64,
	offset float64,
	precondition RevisionPrecondition,
) (semantics.Configuration, error) {
	if err := service.mayMutate(actor); err != nil {
		return semantics.Configuration{}, err
	}
	if err := validateResourceRef(signalRef, "sig_"); err != nil {
		return semantics.Configuration{}, err
	}
	calibration := semantics.Calibration{Scale: scale, Offset: offset}
	if err := calibration.Validate(); err != nil {
		return semantics.Configuration{}, err
	}
	return service.repository.UpdateSignalCalibration(
		ctx, actor, signalRef, scale, offset, precondition,
	)
}

func (service *SemanticConfigurationService) CreateRule(
	ctx context.Context,
	actor Actor,
	signalRef string,
	displayName string,
	spec semantics.RuleSpec,
	precondition RevisionPrecondition,
) (semantics.Rule, error) {
	if err := service.mayMutate(actor); err != nil {
		return semantics.Rule{}, err
	}
	if err := validateResourceRef(signalRef, "sig_"); err != nil {
		return semantics.Rule{}, err
	}
	if err := spec.Validate(); err != nil {
		return semantics.Rule{}, err
	}
	return service.repository.CreateSemanticRule(
		ctx, actor, signalRef, displayName, spec, precondition,
	)
}

func (service *SemanticConfigurationService) UpdateRule(
	ctx context.Context,
	actor Actor,
	ruleID string,
	displayName string,
	spec semantics.RuleSpec,
	precondition RevisionPrecondition,
) (semantics.Rule, error) {
	if err := service.mayMutate(actor); err != nil {
		return semantics.Rule{}, err
	}
	if err := validateResourceRef(ruleID, "rule_"); err != nil {
		return semantics.Rule{}, err
	}
	if err := spec.Validate(); err != nil {
		return semantics.Rule{}, err
	}
	return service.repository.UpdateSemanticRule(
		ctx, actor, ruleID, displayName, spec, precondition,
	)
}

func (service *SemanticConfigurationService) RetireRule(
	ctx context.Context,
	actor Actor,
	ruleID string,
	precondition RevisionPrecondition,
) (semantics.Rule, error) {
	if err := service.mayMutate(actor); err != nil {
		return semantics.Rule{}, err
	}
	if err := validateResourceRef(ruleID, "rule_"); err != nil {
		return semantics.Rule{}, err
	}
	return service.repository.RetireSemanticRule(
		ctx, actor, ruleID, precondition,
	)
}

func (service *SemanticConfigurationService) RequestCounterReset(
	ctx context.Context,
	actor Actor,
	ruleID string,
	resetID string,
) (semantics.CounterReset, error) {
	if err := service.mayMutate(actor); err != nil {
		return semantics.CounterReset{}, err
	}
	if err := validateResourceRef(ruleID, "rule_"); err != nil {
		return semantics.CounterReset{}, err
	}
	if len(resetID) < 1 || len(resetID) > 128 ||
		strings.IndexFunc(resetID, unicode.IsControl) >= 0 {
		return semantics.CounterReset{}, errors.New("invalid semantic counter reset id")
	}
	return service.repository.RequestSemanticCounterReset(
		ctx, actor, ruleID, resetID,
	)
}

func (service *SemanticConfigurationService) ready(actor Actor) error {
	if service == nil || service.repository == nil {
		return errors.New("Site semantic configuration repository is nil")
	}
	return actor.Validate()
}

func (service *SemanticConfigurationService) mayMutate(actor Actor) error {
	if err := service.ready(actor); err != nil {
		return err
	}
	if actor.Class == ActorLocalCLI {
		return nil
	}
	if actor.Class != ActorAccount ||
		(actor.Role != AccountRoleAdmin && actor.Role != AccountRoleSystemAdmin) {
		return ErrForbidden
	}
	return nil
}
