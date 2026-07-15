package contract

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math"
	"strings"
	"unicode"
)

const MaxDescriptorBytes = 1024 * 1024

type DescriptorSnapshot struct {
	SchemaVersion      uint32             `json:"schema_version"`
	EdgeNodeID         string             `json:"edge_node_id"`
	LedgerEpoch        string             `json:"ledger_epoch"`
	DescriptorRevision uint64             `json:"descriptor_revision"`
	Complete           bool               `json:"complete"`
	Devices            []DescriptorDevice `json:"devices"`
	Signals            []DescriptorSignal `json:"signals"`
}

type DescriptorDevice struct {
	SystemID   string  `json:"system_id"`
	Identifier *string `json:"identifier,omitempty"`
	State      string  `json:"state"`
}

type DescriptorSignal struct {
	SeriesKey      string  `json:"series_key"`
	SystemID       string  `json:"system_id"`
	MeasurementKey string  `json:"measurement_key"`
	ChannelIndex   *int32  `json:"channel_index"`
	Variant        string  `json:"variant"`
	Unit           *string `json:"unit"`
	ValueType      string  `json:"value_type"`
}

func DecodeDescriptorSnapshot(payload []byte) (DescriptorSnapshot, error) {
	var snapshot DescriptorSnapshot
	if len(payload) > MaxDescriptorBytes {
		return snapshot, descriptorInvalid("snapshot exceeds encoded byte limit")
	}
	if err := decodeOneStrict(payload, &snapshot); err != nil {
		return snapshot, fmt.Errorf("decode descriptor snapshot: %w", err)
	}
	if err := snapshot.Validate(); err != nil {
		return snapshot, err
	}
	return snapshot, nil
}

func (snapshot DescriptorSnapshot) Validate() error {
	if snapshot.SchemaVersion != SchemaVersion || !snapshot.Complete {
		return descriptorInvalid("only complete descriptor schema version 1 is supported")
	}
	if err := validateTopicSegment("edge_node_id", snapshot.EdgeNodeID); err != nil {
		return err
	}
	if err := validateIdentityComponent("ledger_epoch", snapshot.LedgerEpoch); err != nil {
		return err
	}
	if snapshot.DescriptorRevision < 1 || snapshot.DescriptorRevision > math.MaxInt64 {
		return descriptorInvalid("descriptor_revision must be positive")
	}
	if snapshot.Devices == nil || snapshot.Signals == nil {
		return descriptorInvalid("devices and signals arrays are required")
	}

	deviceIDs := make(map[string]struct{}, len(snapshot.Devices))
	for _, device := range snapshot.Devices {
		if !validUUID(device.SystemID) {
			return descriptorInvalid("device system_id is not a UUID")
		}
		if _, duplicate := deviceIDs[device.SystemID]; duplicate {
			return descriptorInvalid("duplicate device system_id")
		}
		deviceIDs[device.SystemID] = struct{}{}
		if device.State != "quarantined" && device.State != "active" && device.State != "retired" {
			return descriptorInvalid("unsupported device state")
		}
		if device.Identifier != nil && !validDisplayText(*device.Identifier, 64, false) {
			return descriptorInvalid("invalid device identifier")
		}
	}

	seriesKeys := make(map[string]struct{}, len(snapshot.Signals))
	for _, signal := range snapshot.Signals {
		if !validUUID(signal.SystemID) {
			return descriptorInvalid("signal system_id is not a UUID")
		}
		if _, exists := deviceIDs[signal.SystemID]; !exists {
			return descriptorInvalid("signal references an unknown device")
		}
		if signal.MeasurementKey == "" || strings.Contains(signal.MeasurementKey, ":") ||
			signal.Variant == "" || strings.Contains(signal.Variant, ":") ||
			(signal.ChannelIndex != nil && *signal.ChannelIndex < 0) {
			return descriptorInvalid("invalid signal identity")
		}
		channel := "na"
		if signal.ChannelIndex != nil {
			channel = fmt.Sprintf("%d", *signal.ChannelIndex)
		}
		expected := fmt.Sprintf("%s:%s:%s:%s", signal.SystemID, signal.MeasurementKey, channel, signal.Variant)
		if signal.SeriesKey != expected {
			return descriptorInvalid("series_key does not match signal identity")
		}
		if _, duplicate := seriesKeys[signal.SeriesKey]; duplicate {
			return descriptorInvalid("duplicate signal series_key")
		}
		seriesKeys[signal.SeriesKey] = struct{}{}
		if signal.ValueType != "float" && signal.ValueType != "int" && signal.ValueType != "bool" && signal.ValueType != "record" {
			return descriptorInvalid("unsupported signal value_type")
		}
		if signal.Unit != nil && !validDisplayText(*signal.Unit, 128, true) {
			return descriptorInvalid("invalid signal unit")
		}
	}
	return nil
}

func (snapshot DescriptorSnapshot) ContentSHA256() ([sha256.Size]byte, error) {
	if err := snapshot.Validate(); err != nil {
		return [sha256.Size]byte{}, err
	}
	payload, err := json.Marshal(snapshot)
	if err != nil {
		return [sha256.Size]byte{}, err
	}
	return sha256.Sum256(payload), nil
}

func validUUID(value string) bool {
	if len(value) != 36 || value[8] != '-' || value[13] != '-' || value[18] != '-' || value[23] != '-' {
		return false
	}
	hexText := strings.ReplaceAll(value, "-", "")
	decoded, err := hex.DecodeString(hexText)
	return err == nil && len(decoded) == 16
}

func validDisplayText(value string, maxBytes int, allowEmpty bool) bool {
	if len(value) > maxBytes || (!allowEmpty && value == "") {
		return false
	}
	for _, character := range value {
		if unicode.IsControl(character) {
			return false
		}
	}
	return true
}

func descriptorInvalid(message string) error {
	return fmt.Errorf("invalid descriptor v1 message: %s", message)
}
