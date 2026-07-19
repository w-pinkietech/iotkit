package siteapp

import (
	"context"
	"encoding/json"
	"errors"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/outputadapter"
)

type OutputRoute struct {
	RouteID                    string          `json:"route_id"`
	RuleID                     string          `json:"rule_id"`
	AdapterID                  string          `json:"adapter_id"`
	ConfigSchemaVersion        int             `json:"config_schema_version"`
	Config                     json.RawMessage `json:"config"`
	StartAfterObservationRowID int64           `json:"start_after_observation_row_id"`
	Active                     bool            `json:"active"`
	CreatedAt                  int64           `json:"created_at"`
	PendingCount               int64           `json:"pending_count"`
	PublishedCount             int64           `json:"published_count"`
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
