package edgeapp

import (
	"context"
	"encoding/hex"
	"errors"
	"fmt"
	"strings"
	"unicode"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantic"
)

type Operation interface {
	isEdgeOperation()
}

type PutSemanticMapping struct {
	Spec         semantic.MappingSpec
	Precondition RevisionPrecondition
}

func (PutSemanticMapping) isEdgeOperation() {}

type DeactivateSemanticMapping struct {
	EdgeNodeID   string
	SeriesKey    string
	Precondition RevisionPrecondition
}

func (DeactivateSemanticMapping) isEdgeOperation() {}

type PutLegacyMQTTRoute struct {
	MappingID string
	Topic     string
}

func (PutLegacyMQTTRoute) isEdgeOperation() {}

type UpdateDeviceProfile struct {
	DeviceRef    string
	Input        DeviceProfileInput
	Precondition RevisionPrecondition
}

func (UpdateDeviceProfile) isEdgeOperation() {}

type UpdateSignalProfile struct {
	SignalRef    string
	Input        SignalProfileInput
	Precondition RevisionPrecondition
}

func (UpdateSignalProfile) isEdgeOperation() {}

type ActivateEdgeNode struct {
	EdgeNodeRef  string
	Precondition RevisionPrecondition
}

func (ActivateEdgeNode) isEdgeOperation() {}

type CreateEdgeBackup struct {
	Destination string
	Passphrase  string
}

func (CreateEdgeBackup) isEdgeOperation() {}

type AcceptRestoredArchiveLoss struct {
	EdgeNodeID      string
	LedgerEpoch     string
	ConfirmedEdgeID string
	Reason          string
}

func (AcceptRestoredArchiveLoss) isEdgeOperation() {}

type BackupCursor struct {
	EdgeNodeID      string `json:"edge_node_id"`
	LedgerEpoch     string `json:"ledger_epoch"`
	AcceptedThrough int64  `json:"accepted_through"`
}

type BackupManifest struct {
	FormatVersion  int            `json:"format_version"`
	StorageProfile string         `json:"storage_profile,omitempty"`
	PayloadFormat  string         `json:"payload_format,omitempty"`
	BackupID       string         `json:"backup_id"`
	CreatedAt      int64          `json:"created_at"`
	EdgeID         string         `json:"edge_id"`
	SchemaVersion  int            `json:"schema_version"`
	RawRecordCount int64          `json:"raw_record_count"`
	Cursors        []BackupCursor `json:"cursors"`
	DatabaseSHA256 string         `json:"database_sha256"`
}

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
	SemanticMapping     *semantic.Mapping
	LegacyMQTTRoute     *LegacyMQTTRoute
	DeviceProfile       *DeviceProfile
	SignalProfile       *SignalProfile
	EdgeNode            *EdgeNode
	Backup              *BackupManifest
	ArchiveLossAccepted bool
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
	ListEdgeNodes(context.Context) ([]EdgeNode, error)
	RequestEdgeNodeActivation(context.Context, Actor, string, RevisionPrecondition) (EdgeNode, error)
	ApplyEncryptedBackup(context.Context, Actor, string, string) (BackupManifest, error)
	ApplyRestoredArchiveLoss(context.Context, Actor, string, string, string, string) error
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
	case ActivateEdgeNode:
		if actor.Class == ActorAccount &&
			actor.Role != AccountRoleAdmin &&
			actor.Role != AccountRoleSystemAdmin {
			return noResult, ErrForbidden
		}
		if err := validateResourceRef(operation.EdgeNodeRef, "edge_node_"); err != nil {
			return noResult, err
		}
		edgeNode, err := service.repository.RequestEdgeNodeActivation(
			ctx,
			actor,
			operation.EdgeNodeRef,
			operation.Precondition,
		)
		if err != nil {
			return noResult, err
		}
		return Result{EdgeNode: &edgeNode}, nil
	case CreateEdgeBackup:
		if actor.Class != ActorLocalCLI {
			return noResult, ErrForbidden
		}
		if strings.TrimSpace(operation.Destination) == "" ||
			len(operation.Destination) > 4096 ||
			strings.IndexFunc(operation.Destination, unicode.IsControl) >= 0 {
			return noResult, errors.New("backup destination is invalid")
		}
		backup, err := service.repository.ApplyEncryptedBackup(
			ctx, actor, operation.Destination, operation.Passphrase,
		)
		if err != nil {
			return noResult, err
		}
		return Result{Backup: &backup}, nil
	case AcceptRestoredArchiveLoss:
		if actor.Class != ActorLocalCLI {
			return noResult, ErrForbidden
		}
		for name, value := range map[string]string{
			"Edge Node ID":             operation.EdgeNodeID,
			"ledger epoch":             operation.LedgerEpoch,
			"confirmed IoTKit Edge ID": operation.ConfirmedEdgeID,
			"reason":                   operation.Reason,
		} {
			if strings.TrimSpace(value) == "" || len(value) > 512 ||
				strings.IndexFunc(value, unicode.IsControl) >= 0 {
				return noResult, fmt.Errorf("invalid %s", name)
			}
		}
		if err := service.repository.ApplyRestoredArchiveLoss(
			ctx, actor, operation.EdgeNodeID, operation.LedgerEpoch,
			operation.ConfirmedEdgeID, operation.Reason,
		); err != nil {
			return noResult, err
		}
		return Result{ArchiveLossAccepted: true}, nil
	default:
		return noResult, errors.New("unsupported Edge operation")
	}
}

func validateResourceRef(ref, prefix string) error {
	if !strings.HasPrefix(ref, prefix) {
		return errors.New("invalid Edge resource ref")
	}
	randomPart := strings.TrimPrefix(ref, prefix)
	if len(randomPart) != 32 {
		return errors.New("invalid Edge resource ref")
	}
	if _, err := hex.DecodeString(randomPart); err != nil {
		return errors.New("invalid Edge resource ref")
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

func (service *Service) ListEdgeNodes(ctx context.Context) ([]EdgeNode, error) {
	return service.repository.ListEdgeNodes(ctx)
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
