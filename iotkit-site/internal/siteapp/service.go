package siteapp

import (
	"context"
	"encoding/hex"
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

type UpdateDeviceProfile struct {
	DeviceRef    string
	Input        DeviceProfileInput
	Precondition RevisionPrecondition
}

func (UpdateDeviceProfile) isSiteOperation() {}

type UpdateSignalProfile struct {
	SignalRef    string
	Input        SignalProfileInput
	Precondition RevisionPrecondition
}

func (UpdateSignalProfile) isSiteOperation() {}

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
	DeviceProfile   *DeviceProfile
	SignalProfile   *SignalProfile
}

type Repository interface {
	ApplySemanticMapping(context.Context, Actor, semantic.MappingSpec, RevisionPrecondition) (semantic.Mapping, error)
	DeactivateSemanticMapping(context.Context, Actor, string, string, RevisionPrecondition) (semantic.Mapping, error)
	ListSemanticMappings(context.Context) ([]semantic.Mapping, error)
	ListAuditEvents(context.Context, int) ([]AuditEvent, error)
	ApplyLegacyMQTTRoute(context.Context, Actor, string, string) (LegacyMQTTRoute, error)
	UpdateDeviceProfile(context.Context, Actor, string, DeviceProfileInput, RevisionPrecondition) (DeviceProfile, error)
	UpdateSignalProfile(context.Context, Actor, string, SignalProfileInput, RevisionPrecondition) (SignalProfile, error)
	ListInventoryDevices(context.Context, int, string) ([]DeviceSummary, error)
	ListInventorySignals(context.Context, int, string) ([]SignalSummary, error)
	ListSetupDevices(context.Context, int) ([]SetupDeviceSource, error)
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
	case UpdateDeviceProfile:
		if err := validateResourceRef(operation.DeviceRef, "dev_"); err != nil {
			return noResult, err
		}
		if err := operation.Input.Validate(); err != nil {
			return noResult, err
		}
		profile, err := service.repository.UpdateDeviceProfile(
			ctx,
			actor,
			operation.DeviceRef,
			operation.Input,
			operation.Precondition,
		)
		if err != nil {
			return noResult, err
		}
		return Result{DeviceProfile: &profile}, nil
	case UpdateSignalProfile:
		if err := validateResourceRef(operation.SignalRef, "sig_"); err != nil {
			return noResult, err
		}
		if err := operation.Input.Validate(); err != nil {
			return noResult, err
		}
		profile, err := service.repository.UpdateSignalProfile(
			ctx,
			actor,
			operation.SignalRef,
			operation.Input,
			operation.Precondition,
		)
		if err != nil {
			return noResult, err
		}
		return Result{SignalProfile: &profile}, nil
	default:
		return noResult, errors.New("unsupported Site operation")
	}
}

func validateResourceRef(ref, prefix string) error {
	if !strings.HasPrefix(ref, prefix) {
		return errors.New("invalid Site resource ref")
	}
	randomPart := strings.TrimPrefix(ref, prefix)
	if len(randomPart) != 32 {
		return errors.New("invalid Site resource ref")
	}
	if _, err := hex.DecodeString(randomPart); err != nil {
		return errors.New("invalid Site resource ref")
	}
	return nil
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

func (service *Service) ListDevices(ctx context.Context, page PageRequest) ([]DeviceSummary, error) {
	if err := validateInventoryPageRequest(page, "dev_"); err != nil {
		return nil, err
	}
	return service.repository.ListInventoryDevices(ctx, page.Limit, page.AfterRef)
}

func (service *Service) ListSignals(ctx context.Context, page PageRequest) ([]SignalSummary, error) {
	if err := validateInventoryPageRequest(page, "sig_"); err != nil {
		return nil, err
	}
	return service.repository.ListInventorySignals(ctx, page.Limit, page.AfterRef)
}

func validateInventoryPageRequest(page PageRequest, prefix string) error {
	if page.Limit < 1 || page.Limit > 100 {
		return errors.New("inventory page limit must be between 1 and 100")
	}
	if page.AfterRef != "" {
		return validateResourceRef(page.AfterRef, prefix)
	}
	return nil
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
