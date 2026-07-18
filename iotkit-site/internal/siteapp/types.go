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
	ErrForbidden        = errors.New("Site operation is forbidden")
	ErrAlreadyOwned     = errors.New("Site already has an owner")
	ErrLastSystemAdmin  = errors.New("the last active system administrator cannot be changed")
)

type ActorClass string

const (
	ActorLocalCLI        ActorClass = "local_cli"
	ActorSettingsSession ActorClass = "settings_session"
	ActorAccount         ActorClass = "account"
	ActorSystem          ActorClass = "system"
)

type Actor struct {
	Class ActorClass
	Ref   string
	Role  AccountRole
}

func LocalCLIActor() Actor {
	return Actor{Class: ActorLocalCLI, Ref: "local_cli"}
}

func AccountActor(accountRef string, role AccountRole) Actor {
	return Actor{Class: ActorAccount, Ref: accountRef, Role: role}
}

func (actor Actor) Validate() error {
	if actor.Class != ActorLocalCLI && actor.Class != ActorSettingsSession &&
		actor.Class != ActorAccount && actor.Class != ActorSystem {
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
	if actor.Class == ActorAccount {
		if err := validateResourceRef(actor.Ref, "acct_"); err != nil {
			return err
		}
		if !actor.Role.Valid() {
			return errors.New("Site account actor role is invalid")
		}
	}
	return nil
}

type RevisionPrecondition struct {
	Expected *int64
}

type EdgeState string

const (
	EdgeDiscovered   EdgeState = "discovered"
	EdgeActivating   EdgeState = "activating"
	EdgeActive       EdgeState = "active"
	EdgeRecoveryHold EdgeState = "recovery_hold"
)

type Edge struct {
	EdgeRef          string    `json:"edge_ref"`
	EdgeNodeID       string    `json:"edge_node_id"`
	LedgerEpoch      string    `json:"ledger_epoch"`
	State            EdgeState `json:"state"`
	ActivationID     string    `json:"-"`
	GrantRevision    uint64    `json:"-"`
	DisplayName      string    `json:"display_name"`
	Location         string    `json:"location"`
	DeviceCount      int64     `json:"device_count"`
	SensorCount      int64     `json:"sensor_count"`
	Revision         int64     `json:"revision"`
	LastDescriptorAt *int64    `json:"last_descriptor_at,omitempty"`
	LastResultAt     *int64    `json:"last_result_at,omitempty"`
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
	DisplayName            string `json:"display_name"`
	DisplaySensorType      string `json:"display_sensor_type"`
	DisplaySensorTypeLabel string `json:"display_sensor_type_label"`
	DisplayValueKind       string `json:"display_value_kind"`
	DisplayUnitMode        string `json:"display_unit_mode"`
	DisplayUnit            string `json:"display_unit"`
	DecimalPlaces          int    `json:"decimal_places"`
}

func (input SignalProfileInput) Validate() error {
	if err := validateProfileText("display name", input.DisplayName, 128); err != nil {
		return err
	}
	switch input.DisplaySensorType {
	case "temperature", "contact", "illuminance", "distance", "voltage",
		"current", "pressure", "humidity", "acceleration":
	case "custom":
		if err := validateProfileText(
			"custom sensor type label",
			input.DisplaySensorTypeLabel,
			64,
		); err != nil {
			return err
		}
	default:
		return errors.New("display sensor type is invalid")
	}
	if input.DisplaySensorTypeLabel != "" {
		if err := validateOptionalProfileText(
			"custom sensor type label",
			input.DisplaySensorTypeLabel,
			64,
		); err != nil {
			return err
		}
	}
	switch input.DisplayValueKind {
	case "numeric":
	case "boolean":
		if input.DisplayUnitMode != "dimensionless" {
			return errors.New("boolean display value must be dimensionless")
		}
		if strings.TrimSpace(input.DisplayUnit) != "" {
			return errors.New("boolean display value must not have a unit")
		}
		if input.DecimalPlaces != 0 {
			return errors.New("boolean display value must not have decimal places")
		}
	default:
		return errors.New("display value kind is invalid")
	}
	switch input.DisplayUnitMode {
	case "unit":
		if err := validateProfileText("display unit", input.DisplayUnit, 32); err != nil {
			return err
		}
	case "dimensionless":
		if strings.TrimSpace(input.DisplayUnit) != "" {
			return errors.New("dimensionless display value must not have a unit")
		}
	default:
		return errors.New("display unit mode is invalid")
	}
	if input.DecimalPlaces < 0 || input.DecimalPlaces > 6 {
		return errors.New("decimal places must be between 0 and 6")
	}
	return nil
}

type DeviceProfile struct {
	DeviceRef   string `json:"device_ref"`
	DisplayName string `json:"display_name"`
	Location    string `json:"location"`
	Revision    int64  `json:"revision"`
	UpdatedAt   int64  `json:"updated_at"`
}

type SignalProfile struct {
	SignalRef              string `json:"signal_ref"`
	DisplayName            string `json:"display_name"`
	DisplaySensorType      string `json:"display_sensor_type"`
	DisplaySensorTypeLabel string `json:"display_sensor_type_label"`
	DisplayValueKind       string `json:"display_value_kind"`
	DisplayUnitMode        string `json:"display_unit_mode"`
	DisplayUnit            string `json:"display_unit"`
	DecimalPlaces          int    `json:"decimal_places"`
	Revision               int64  `json:"revision"`
	UpdatedAt              int64  `json:"updated_at"`
}

func (profile SignalProfile) Complete() bool {
	return (SignalProfileInput{
		DisplayName:            profile.DisplayName,
		DisplaySensorType:      profile.DisplaySensorType,
		DisplaySensorTypeLabel: profile.DisplaySensorTypeLabel,
		DisplayValueKind:       profile.DisplayValueKind,
		DisplayUnitMode:        profile.DisplayUnitMode,
		DisplayUnit:            profile.DisplayUnit,
		DecimalPlaces:          profile.DecimalPlaces,
	}).Validate() == nil
}

type PageRequest struct {
	Limit    int
	AfterRef string
}

type LatestMeasurement struct {
	Values         json.RawMessage `json:"values"`
	EventTime      int64           `json:"event_time"`
	SiteReceivedAt int64           `json:"site_received_at"`
}

type DeviceSummary struct {
	DeviceRef          string  `json:"device_ref"`
	Edge               string  `json:"edge"`
	Identifier         *string `json:"-"`
	DisplayName        string  `json:"display_name"`
	Location           string  `json:"location"`
	ProfileRevision    *int64  `json:"profile_revision"`
	DescriptorPresence string  `json:"descriptor_presence"`
	DeviceState        string  `json:"device_state"`
	LastReceivedAt     *int64  `json:"last_received_at"`
}

type SignalSummary struct {
	SignalRef          string             `json:"signal_ref"`
	SeriesKey          string             `json:"-"`
	Edge               string             `json:"edge"`
	DeviceRef          *string            `json:"device_ref"`
	DisplayName        string             `json:"display_name"`
	ProfileRevision    *int64             `json:"profile_revision"`
	Profile            *SignalProfile     `json:"profile,omitempty"`
	DescriptorPresence string             `json:"descriptor_presence"`
	Unit               *string            `json:"unit"`
	ValueType          *string            `json:"value_type"`
	SensorType         *string            `json:"sensor_type"`
	ChannelIndex       *int32             `json:"channel_index,omitempty"`
	Latest             *LatestMeasurement `json:"latest"`
	LastReceivedAt     *int64             `json:"last_received_at"`
	HasSemanticMapping bool               `json:"has_semantic_mapping"`
	ReceiptStatus      string             `json:"receipt_status"`
}

type SetupSignalSource struct {
	Signal       SignalSummary
	ChannelIndex *int32
	Profile      *SignalProfile
}

type SetupDeviceSource struct {
	Device     DeviceSummary
	Identifier *string
	Signals    []SetupSignalSource
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

func validateOptionalProfileText(name, value string, maxBytes int) error {
	if len(strings.TrimSpace(value)) > maxBytes {
		return errors.New(name + " is too long")
	}
	if strings.IndexFunc(value, unicode.IsControl) >= 0 {
		return errors.New(name + " must not contain control characters")
	}
	return nil
}

type AuditEvent struct {
	AuditRowID       int64           `json:"audit_row_id"`
	OccurredAt       int64           `json:"occurred_at"`
	ActorClass       ActorClass      `json:"actor_class"`
	ActorRef         string          `json:"actor_ref"`
	ActorLoginID     *string         `json:"actor_login_id"`
	ActorDisplayName *string         `json:"actor_display_name"`
	Operation        string          `json:"operation"`
	ResourceRef      string          `json:"resource_ref"`
	Outcome          string          `json:"outcome"`
	Summary          json.RawMessage `json:"summary"`
}
