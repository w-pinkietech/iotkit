package siteapp

import (
	"context"
	"encoding/json"
	"errors"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/outputadapter"
)

type ExportProfileState string

const (
	ExportProfilePreparing ExportProfileState = "preparing"
	ExportProfileActive    ExportProfileState = "active"
	ExportProfileDraining  ExportProfileState = "draining"
	ExportProfileStopped   ExportProfileState = "stopped"
)

type OutputBindingState string

const (
	OutputBindingNeedsConfiguration OutputBindingState = "needs_configuration"
	OutputBindingPrepared           OutputBindingState = "prepared"
	OutputBindingActive             OutputBindingState = "active"
	OutputBindingIneligible         OutputBindingState = "ineligible"
	OutputBindingDraining           OutputBindingState = "draining"
	OutputBindingStopped            OutputBindingState = "stopped"
)

type OutputProfileRuleBinding struct {
	BindingID        string             `json:"binding_id"`
	ProfileID        string             `json:"profile_id"`
	RuleID           string             `json:"rule_id"`
	OutputIdentityID string             `json:"output_identity_id,omitempty"`
	RuleDisplayName  string             `json:"rule_display_name,omitempty"`
	RuleKind         string             `json:"rule_kind,omitempty"`
	SignalRef        string             `json:"signal_ref,omitempty"`
	SensorName       string             `json:"sensor_name,omitempty"`
	SourceID         string             `json:"source_id"`
	SignalID         string             `json:"signal_id,omitempty"`
	Mode             string             `json:"mode,omitempty"`
	Reason           string             `json:"reason,omitempty"`
	State            OutputBindingState `json:"state"`
	IneligibleReason string             `json:"ineligible_reason,omitempty"`
	Revision         int64              `json:"revision"`
	CreatedAt        int64              `json:"created_at"`
	ActivatedAt      *int64             `json:"activated_at,omitempty"`
	StoppedAt        *int64             `json:"stopped_at,omitempty"`
}

type ExportProfile struct {
	ProfileID            string                     `json:"profile_id"`
	DisplayName          string                     `json:"display_name"`
	AdapterID            string                     `json:"adapter_id"`
	AdapterSchemaVersion int                        `json:"adapter_schema_version"`
	State                ExportProfileState         `json:"state"`
	AutoBindFutureRules  bool                       `json:"auto_bind_future_rules"`
	Revision             int64                      `json:"revision"`
	CreatedAt            int64                      `json:"created_at"`
	DrainRequestedAt     *int64                     `json:"drain_requested_at,omitempty"`
	StoppedAt            *int64                     `json:"stopped_at,omitempty"`
	Bindings             []OutputProfileRuleBinding `json:"bindings"`
}

type ExportProfileActivationPreview struct {
	AdapterID               string                        `json:"adapter_id"`
	AutomaticCount          int                           `json:"automatic_count"`
	NeedsConfigurationCount int                           `json:"needs_configuration_count"`
	IneligibleCount         int                           `json:"ineligible_count"`
	Rules                   []OutputActivationRulePreview `json:"rules"`
}

type OutputActivationRulePreview struct {
	RuleID      string `json:"rule_id"`
	DisplayName string `json:"display_name"`
	SensorName  string `json:"sensor_name"`
	Kind        string `json:"kind"`
	Disposition string `json:"disposition"`
}

type OutputPublicationPreview struct {
	BindingID  string          `json:"binding_id"`
	Provenance string          `json:"provenance"`
	Topic      string          `json:"topic"`
	QoS        byte            `json:"qos"`
	Retain     bool            `json:"retain"`
	Payload    json.RawMessage `json:"payload"`
}

type OutputRoute struct {
	RouteID                    string          `json:"route_id"`
	BindingID                  string          `json:"binding_id,omitempty"`
	RuleID                     string          `json:"rule_id"`
	AdapterID                  string          `json:"adapter_id"`
	ConfigSchemaVersion        int             `json:"config_schema_version"`
	Config                     json.RawMessage `json:"config"`
	StartAfterObservationRowID int64           `json:"start_after_observation_row_id"`
	Active                     bool            `json:"active"`
	LifecycleState             string          `json:"lifecycle_state"`
	CreatedAt                  int64           `json:"created_at"`
	PendingCount               int64           `json:"pending_count"`
	PublishedCount             int64           `json:"published_count"`
	LastTransformErrorCode     string          `json:"last_transform_error_code,omitempty"`
	LastTransformErrorAt       *int64          `json:"last_transform_error_at,omitempty"`
	LastTransformSuccessAt     *int64          `json:"last_transform_success_at,omitempty"`
	LastPublishedAt            *int64          `json:"last_published_at,omitempty"`
	OldestPendingAt            *int64          `json:"oldest_pending_at,omitempty"`
	SignalRef                  string          `json:"signal_ref,omitempty"`
	RuleDisplayName            string          `json:"rule_display_name,omitempty"`
	RuleKind                   string          `json:"rule_kind,omitempty"`
}

type YokaKitRuleRoute struct {
	RouteID                    string                    `json:"route_id"`
	RuleID                     string                    `json:"rule_id"`
	SourceID                   string                    `json:"source_id"`
	SignalID                   string                    `json:"signal_id"`
	Kind                       outputadapter.YokaKitKind `json:"kind"`
	Reason                     string                    `json:"reason,omitempty"`
	StartAfterObservationRowID int64                     `json:"start_after_observation_row_id"`
	Active                     bool                      `json:"active"`
	CreatedAt                  int64                     `json:"created_at"`
	PendingCount               int64                     `json:"pending_count"`
	PublishedCount             int64                     `json:"published_count"`
}

type RuleOutputRepository interface {
	ApplyOutputRoute(
		context.Context,
		Actor,
		string,
		string,
		json.RawMessage,
	) (OutputRoute, error)
}

type RuleOutputService struct {
	repository RuleOutputRepository
}

func NewRuleOutputService(repository RuleOutputRepository) *RuleOutputService {
	return &RuleOutputService{repository: repository}
}

func (service *RuleOutputService) CreateYokaKitRoute(
	ctx context.Context,
	actor Actor,
	ruleID string,
	config outputadapter.YokaKitConfig,
) (YokaKitRuleRoute, error) {
	var noRoute YokaKitRuleRoute
	encoded, err := outputadapter.EncodeYokaKitConfig(config)
	if err != nil {
		return noRoute, err
	}
	route, err := service.CreateOutputRoute(
		ctx,
		actor,
		ruleID,
		"yokakit.mqtt.v1",
		encoded,
	)
	if err != nil {
		return noRoute, err
	}
	return YokaKitRuleRoute{
		RouteID:                    route.RouteID,
		RuleID:                     route.RuleID,
		SourceID:                   config.SourceID,
		SignalID:                   config.SignalID,
		Kind:                       config.Kind,
		Reason:                     config.Reason,
		StartAfterObservationRowID: route.StartAfterObservationRowID,
		Active:                     route.Active,
		CreatedAt:                  route.CreatedAt,
		PendingCount:               route.PendingCount,
		PublishedCount:             route.PublishedCount,
	}, nil
}

func (service *RuleOutputService) CreateOutputRoute(
	ctx context.Context,
	actor Actor,
	ruleID string,
	adapterID string,
	config json.RawMessage,
) (OutputRoute, error) {
	var noRoute OutputRoute
	if service == nil || service.repository == nil {
		return noRoute, errors.New("Site output route repository is nil")
	}
	if err := actor.Validate(); err != nil {
		return noRoute, err
	}
	if actor.Class != ActorLocalCLI &&
		(actor.Class != ActorAccount ||
			(actor.Role != AccountRoleAdmin &&
				actor.Role != AccountRoleSystemAdmin)) {
		return noRoute, ErrForbidden
	}
	if err := validateResourceRef(ruleID, "rule_"); err != nil {
		return noRoute, err
	}
	return service.repository.ApplyOutputRoute(
		ctx,
		actor,
		ruleID,
		adapterID,
		config,
	)
}
