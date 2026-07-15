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

type DeviceProfileInput struct {
	DisplayName string
	Location    string
}

func (input DeviceProfileInput) Validate() error {
	if err := validateProfileText("display name", input.DisplayName, 128); err != nil {
		return err
	}
	return validateProfileText("location", input.Location, 256)
}

type SignalProfileInput struct {
	DisplayName string
}

func (input SignalProfileInput) Validate() error {
	return validateProfileText("display name", input.DisplayName, 128)
}

type DeviceProfile struct {
	DeviceRef   string `json:"device_ref"`
	DisplayName string `json:"display_name"`
	Location    string `json:"location"`
	Revision    int64  `json:"revision"`
	UpdatedAt   int64  `json:"updated_at"`
}

type SignalProfile struct {
	SignalRef   string `json:"signal_ref"`
	DisplayName string `json:"display_name"`
	Revision    int64  `json:"revision"`
	UpdatedAt   int64  `json:"updated_at"`
}

func validateProfileText(name, value string, maxBytes int) error {
	trimmed := strings.TrimSpace(value)
	if trimmed == "" {
		return errors.New(name + " must not be empty")
	}
	if len(trimmed) > maxBytes {
		return errors.New(name + " is too long")
	}
	if strings.IndexFunc(value, unicode.IsControl) >= 0 {
		return errors.New(name + " must not contain control characters")
	}
	return nil
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
