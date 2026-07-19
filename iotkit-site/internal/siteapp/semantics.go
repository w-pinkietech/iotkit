package siteapp

import (
	"context"
	"errors"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantics"
)

type SemanticRepository interface {
	ApplySemanticDefinition(
		context.Context,
		Actor,
		string,
		semantics.DefinitionSpec,
		RevisionPrecondition,
	) (semantics.Definition, error)
	DeactivateSemanticDefinition(
		context.Context,
		Actor,
		string,
		RevisionPrecondition,
	) (semantics.Definition, error)
	ResetSemanticCounter(
		context.Context,
		Actor,
		string,
		RevisionPrecondition,
	) (semantics.Definition, error)
	ListSemanticDefinitions(context.Context) ([]semantics.Definition, error)
}

type SemanticService struct {
	repository SemanticRepository
}

func NewSemanticService(repository SemanticRepository) *SemanticService {
	return &SemanticService{repository: repository}
}

func (service *SemanticService) Put(
	ctx context.Context,
	actor Actor,
	signalRef string,
	spec semantics.DefinitionSpec,
	precondition RevisionPrecondition,
) (semantics.Definition, error) {
	if actor.Class != ActorAccount ||
		(actor.Role != AccountRoleAdmin && actor.Role != AccountRoleSystemAdmin) {
		return semantics.Definition{}, ErrForbidden
	}
	if err := validateResourceRef(signalRef, "sig_"); err != nil {
		return semantics.Definition{}, err
	}
	if err := spec.Validate(); err != nil {
		return semantics.Definition{}, err
	}
	return service.repository.ApplySemanticDefinition(
		ctx, actor, signalRef, spec, precondition,
	)
}

func (service *SemanticService) Deactivate(
	ctx context.Context,
	actor Actor,
	signalRef string,
	precondition RevisionPrecondition,
) (semantics.Definition, error) {
	if actor.Class != ActorAccount ||
		(actor.Role != AccountRoleAdmin && actor.Role != AccountRoleSystemAdmin) {
		return semantics.Definition{}, ErrForbidden
	}
	if err := validateResourceRef(signalRef, "sig_"); err != nil {
		return semantics.Definition{}, err
	}
	return service.repository.DeactivateSemanticDefinition(
		ctx, actor, signalRef, precondition,
	)
}

func (service *SemanticService) ResetCounter(
	ctx context.Context,
	actor Actor,
	signalRef string,
	precondition RevisionPrecondition,
) (semantics.Definition, error) {
	if actor.Class != ActorAccount ||
		(actor.Role != AccountRoleAdmin && actor.Role != AccountRoleSystemAdmin) {
		return semantics.Definition{}, ErrForbidden
	}
	if err := validateResourceRef(signalRef, "sig_"); err != nil {
		return semantics.Definition{}, err
	}
	return service.repository.ResetSemanticCounter(
		ctx, actor, signalRef, precondition,
	)
}

func (service *SemanticService) List(ctx context.Context) ([]semantics.Definition, error) {
	if service == nil || service.repository == nil {
		return nil, errors.New("Site semantic repository is nil")
	}
	return service.repository.ListSemanticDefinitions(ctx)
}
