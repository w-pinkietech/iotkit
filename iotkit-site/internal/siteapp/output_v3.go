package siteapp

import (
	"context"
	"errors"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/outputadapter"
)

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
	ApplyYokaKitRuleRoute(
		context.Context,
		Actor,
		string,
		outputadapter.YokaKit,
	) (YokaKitRuleRoute, error)
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
	adapter outputadapter.YokaKit,
) (YokaKitRuleRoute, error) {
	var noRoute YokaKitRuleRoute
	if service == nil || service.repository == nil {
		return noRoute, errors.New("Site rule output repository is nil")
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
	if err := adapter.Validate(); err != nil {
		return noRoute, err
	}
	return service.repository.ApplyYokaKitRuleRoute(
		ctx,
		actor,
		ruleID,
		adapter,
	)
}
