package edgeapp

import (
	"context"
	"encoding/json"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/outputadapter"
)

type ruleOutputRepositoryStub struct {
	called bool
}

func (stub *ruleOutputRepositoryStub) ApplyOutputRoute(
	context.Context,
	Actor,
	string,
	string,
	json.RawMessage,
) (OutputRoute, error) {
	stub.called = true
	return OutputRoute{
		RouteID: "out_0123456789abcdef0123456789abcdef",
		RuleID:  "rule_0123456789abcdef0123456789abcdef",
	}, nil
}

func TestRuleOutputServiceRequiresAdminForMutation(t *testing.T) {
	stub := &ruleOutputRepositoryStub{}
	service := NewRuleOutputService(stub)
	viewer := AccountActor(
		"acct_0123456789abcdef0123456789abcdef",
		AccountRoleViewer,
	)
	adapter := outputadapter.YokaKitConfig{
		SourceID: "line-a",
		SignalID: "production",
		Kind:     outputadapter.YokaKitProduction,
	}
	if _, err := service.CreateYokaKitRoute(
		context.Background(),
		viewer,
		"rule_0123456789abcdef0123456789abcdef",
		adapter,
	); err != ErrForbidden {
		t.Fatalf("viewer create error=%v", err)
	}
	admin := AccountActor(
		"acct_0123456789abcdef0123456789abcdef",
		AccountRoleAdmin,
	)
	if _, err := service.CreateYokaKitRoute(
		context.Background(),
		admin,
		"rule_0123456789abcdef0123456789abcdef",
		adapter,
	); err != nil || !stub.called {
		t.Fatalf("admin create called=%t err=%v", stub.called, err)
	}
}
