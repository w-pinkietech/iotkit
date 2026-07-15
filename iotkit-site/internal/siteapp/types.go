package siteapp

import (
	"encoding/json"
	"errors"
	"strings"
	"unicode"
)

var (
	ErrNotFound         = errors.New("Site resource not found")
	ErrRevisionMismatch = errors.New("Site resource revision mismatch")
)

type ActorClass string

const (
	ActorLocalCLI        ActorClass = "local_cli"
	ActorSettingsSession ActorClass = "settings_session"
	ActorSystem          ActorClass = "system"
)

type Actor struct {
	Class ActorClass
	Ref   string
}

func LocalCLIActor() Actor {
	return Actor{Class: ActorLocalCLI, Ref: "local_cli"}
}

func (actor Actor) Validate() error {
	if actor.Class != ActorLocalCLI && actor.Class != ActorSettingsSession && actor.Class != ActorSystem {
		return errors.New("unsupported Site actor class")
	}
	if strings.TrimSpace(actor.Ref) == "" {
		return errors.New("Site actor ref must not be empty")
	}
	if len(actor.Ref) > 128 {
		return errors.New("Site actor ref must not exceed 128 bytes")
	}
	if strings.IndexFunc(actor.Ref, unicode.IsControl) >= 0 {
		return errors.New("Site actor ref must not contain control characters")
	}
	return nil
}

type RevisionPrecondition struct {
	Expected *int64
}

type AuditEvent struct {
	AuditRowID  int64           `json:"audit_row_id"`
	OccurredAt  int64           `json:"occurred_at"`
	ActorClass  ActorClass      `json:"actor_class"`
	ActorRef    string          `json:"actor_ref"`
	Operation   string          `json:"operation"`
	ResourceRef string          `json:"resource_ref"`
	Outcome     string          `json:"outcome"`
	Summary     json.RawMessage `json:"summary"`
}
