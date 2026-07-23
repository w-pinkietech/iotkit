package edgehttp

import (
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"strings"
)

type consoleAuditView struct {
	OccurredAt   string
	Actor        string
	Operation    string
	Resource     string
	Outcome      string
	OutcomeClass string
}

func newConsoleAuditViews(events []edgeapp.AuditEvent) []consoleAuditView {
	views := make([]consoleAuditView, 0, len(events))
	for _, event := range events {
		if strings.HasPrefix(event.Operation, "session.") {
			continue
		}
		actor := displayActor(event.ActorClass)
		if event.ActorDisplayName != nil && *event.ActorDisplayName != "" {
			actor = *event.ActorDisplayName
		}
		outcomeClass := "failed"
		if event.Outcome == "success" {
			outcomeClass = "configured"
		}
		views = append(views, consoleAuditView{
			OccurredAt:   displayDateTime(event.OccurredAt),
			Actor:        actor,
			Operation:    displayOperation(event.Operation),
			Resource:     displayResource(event.ResourceRef),
			Outcome:      displayOutcome(event.Outcome),
			OutcomeClass: outcomeClass,
		})
	}
	return views
}
