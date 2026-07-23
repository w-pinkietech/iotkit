package edgeapp

import (
	"context"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/semantics"
)

type semanticV3RepositoryStub struct {
	configuration semantics.Configuration
	created       semantics.Rule
}

func (stub *semanticV3RepositoryStub) GetSemanticConfiguration(
	context.Context,
	string,
) (semantics.Configuration, error) {
	return stub.configuration, nil
}

func (stub *semanticV3RepositoryStub) UpdateSignalCalibration(
	context.Context,
	Actor,
	string,
	float64,
	float64,
	RevisionPrecondition,
) (semantics.Configuration, error) {
	return stub.configuration, nil
}

func (stub *semanticV3RepositoryStub) CreateSemanticRule(
	context.Context,
	Actor,
	string,
	string,
	semantics.RuleSpec,
	RevisionPrecondition,
) (semantics.Rule, error) {
	return stub.created, nil
}

func (stub *semanticV3RepositoryStub) UpdateSemanticRule(
	context.Context,
	Actor,
	string,
	string,
	semantics.RuleSpec,
	RevisionPrecondition,
) (semantics.Rule, error) {
	return stub.created, nil
}

func (stub *semanticV3RepositoryStub) RetireSemanticRule(
	context.Context,
	Actor,
	string,
	RevisionPrecondition,
) (semantics.Rule, error) {
	return stub.created, nil
}

func (stub *semanticV3RepositoryStub) RequestSemanticCounterReset(
	context.Context,
	Actor,
	string,
	string,
) (semantics.CounterReset, error) {
	return semantics.CounterReset{}, nil
}

func TestSemanticConfigurationServiceAllowsViewerReadAndAdminMutation(t *testing.T) {
	stub := &semanticV3RepositoryStub{
		configuration: semantics.Configuration{
			SignalRef: "sig_0123456789abcdef0123456789abcdef",
			Revision:  1,
		},
		created: semantics.Rule{
			ID:       "rule_0123456789abcdef0123456789abcdef",
			Revision: 1,
		},
	}
	service := NewSemanticConfigurationService(stub)
	viewer := Actor{
		Class: ActorAccount, Ref: "acct_0123456789abcdef0123456789abcdef",
		Role: AccountRoleViewer,
	}
	if _, err := service.Get(
		context.Background(),
		viewer,
		stub.configuration.SignalRef,
	); err != nil {
		t.Fatal(err)
	}
	if _, err := service.CreateRule(
		context.Background(),
		viewer,
		stub.configuration.SignalRef,
		"回数",
		semantics.RuleSpec{Kind: semantics.KindNumeric},
		RevisionPrecondition{},
	); err != ErrForbidden {
		t.Fatalf("viewer mutation error = %v", err)
	}
	admin := viewer
	admin.Role = AccountRoleAdmin
	if _, err := service.CreateRule(
		context.Background(),
		admin,
		stub.configuration.SignalRef,
		"回数",
		semantics.RuleSpec{Kind: semantics.KindNumeric},
		RevisionPrecondition{},
	); err != nil {
		t.Fatal(err)
	}
}
