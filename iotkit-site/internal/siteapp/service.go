package siteapp

import (
	"context"
	"errors"
	"strings"
	"unicode"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantic"
)

type Operation interface {
	isSiteOperation()
}

type PutSemanticMapping struct {
	Spec         semantic.MappingSpec
	Precondition RevisionPrecondition
}

func (PutSemanticMapping) isSiteOperation() {}

type DeactivateSemanticMapping struct {
	EdgeNodeID   string
	SeriesKey    string
	Precondition RevisionPrecondition
}

func (DeactivateSemanticMapping) isSiteOperation() {}

type Result struct {
	SemanticMapping *semantic.Mapping
}

type Repository interface {
	ApplySemanticMapping(context.Context, Actor, semantic.MappingSpec, RevisionPrecondition) (semantic.Mapping, error)
	DeactivateSemanticMapping(context.Context, Actor, string, string, RevisionPrecondition) (semantic.Mapping, error)
	ListSemanticMappings(context.Context) ([]semantic.Mapping, error)
	ListAuditEvents(context.Context, int) ([]AuditEvent, error)
}

type Service struct {
	repository Repository
}

func NewService(repository Repository) *Service {
	return &Service{repository: repository}
}

func (service *Service) Dispatch(ctx context.Context, actor Actor, operation Operation) (Result, error) {
	var noResult Result
	if err := actor.Validate(); err != nil {
		return noResult, err
	}

	switch operation := operation.(type) {
	case PutSemanticMapping:
		if err := operation.Spec.Validate(); err != nil {
			return noResult, err
		}
		mapping, err := service.repository.ApplySemanticMapping(
			ctx,
			actor,
			operation.Spec,
			operation.Precondition,
		)
		if err != nil {
			return noResult, err
		}
		return Result{SemanticMapping: &mapping}, nil
	case DeactivateSemanticMapping:
		if err := validateSourceIdentity(operation.EdgeNodeID, operation.SeriesKey); err != nil {
			return noResult, err
		}
		mapping, err := service.repository.DeactivateSemanticMapping(
			ctx,
			actor,
			operation.EdgeNodeID,
			operation.SeriesKey,
			operation.Precondition,
		)
		if err != nil {
			return noResult, err
		}
		return Result{SemanticMapping: &mapping}, nil
	default:
		return noResult, errors.New("unsupported Site operation")
	}
}

func (service *Service) ListSemanticMappings(ctx context.Context) ([]semantic.Mapping, error) {
	return service.repository.ListSemanticMappings(ctx)
}

func (service *Service) ListAuditEvents(ctx context.Context, limit int) ([]AuditEvent, error) {
	if limit < 1 || limit > 100 {
		return nil, errors.New("audit event limit must be between 1 and 100")
	}
	return service.repository.ListAuditEvents(ctx, limit)
}

func validateSourceIdentity(edgeNodeID, seriesKey string) error {
	if strings.TrimSpace(edgeNodeID) == "" {
		return errors.New("edge_node_id must not be empty")
	}
	if strings.ContainsAny(edgeNodeID, "/+#") {
		return errors.New("edge_node_id must not contain /, +, or #")
	}
	if strings.TrimSpace(seriesKey) == "" {
		return errors.New("series_key must not be empty")
	}
	if strings.IndexFunc(edgeNodeID, unicode.IsControl) >= 0 || strings.IndexFunc(seriesKey, unicode.IsControl) >= 0 {
		return errors.New("source identity must not contain control characters")
	}
	return nil
}
