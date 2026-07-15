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

type PutLegacyMQTTRoute struct {
	MappingID string
	Topic     string
}

func (PutLegacyMQTTRoute) isSiteOperation() {}

func (operation PutLegacyMQTTRoute) Validate() error {
	return validateLegacyMQTTRoute(operation.MappingID, operation.Topic)
}

type LegacyMQTTRoute struct {
	RouteID              string `json:"route_id"`
	MappingID            string `json:"mapping_id"`
	Topic                string `json:"topic"`
	QoS                  int    `json:"qos"`
	StartAfterEventRowID int64  `json:"start_after_event_row_id"`
	Active               bool   `json:"active"`
	CreatedAt            int64  `json:"created_at"`
}

type Result struct {
	SemanticMapping *semantic.Mapping
	LegacyMQTTRoute *LegacyMQTTRoute
}

type Repository interface {
	ApplySemanticMapping(context.Context, Actor, semantic.MappingSpec, RevisionPrecondition) (semantic.Mapping, error)
	DeactivateSemanticMapping(context.Context, Actor, string, string, RevisionPrecondition) (semantic.Mapping, error)
	ListSemanticMappings(context.Context) ([]semantic.Mapping, error)
	ListAuditEvents(context.Context, int) ([]AuditEvent, error)
	ApplyLegacyMQTTRoute(context.Context, Actor, string, string) (LegacyMQTTRoute, error)
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
	case PutLegacyMQTTRoute:
		if err := operation.Validate(); err != nil {
			return noResult, err
		}
		route, err := service.repository.ApplyLegacyMQTTRoute(
			ctx,
			actor,
			operation.MappingID,
			operation.Topic,
		)
		if err != nil {
			return noResult, err
		}
		return Result{LegacyMQTTRoute: &route}, nil
	default:
		return noResult, errors.New("unsupported Site operation")
	}
}

func validateLegacyMQTTRoute(mappingID, topic string) error {
	if strings.TrimSpace(mappingID) == "" {
		return errors.New("mapping_id must not be empty")
	}
	if strings.TrimSpace(topic) == "" {
		return errors.New("MQTT topic must not be empty")
	}
	if strings.HasPrefix(topic, "/") || strings.HasSuffix(topic, "/") {
		return errors.New("MQTT topic must not start or end with /")
	}
	if strings.ContainsAny(topic, "+#") {
		return errors.New("MQTT topic must not contain wildcards")
	}
	if strings.IndexFunc(mappingID, unicode.IsControl) >= 0 || strings.IndexFunc(topic, unicode.IsControl) >= 0 {
		return errors.New("legacy MQTT route must not contain control characters")
	}
	return nil
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
