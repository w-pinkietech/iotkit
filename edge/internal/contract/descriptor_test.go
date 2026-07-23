package contract

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestDecodeDescriptorSnapshotFixture(t *testing.T) {
	snapshot, err := DecodeDescriptorSnapshot(descriptorFixture(t))
	if err != nil {
		t.Fatal(err)
	}
	if snapshot.EdgeNodeID != "edge-node-01" || snapshot.DescriptorRevision != 5 || !snapshot.Complete {
		t.Fatalf("snapshot = %#v", snapshot)
	}
	if len(snapshot.Devices) != 1 || snapshot.Devices[0].Identifier == nil || *snapshot.Devices[0].Identifier != "01234567" {
		t.Fatalf("devices = %#v", snapshot.Devices)
	}
	if len(snapshot.Signals) != 1 || snapshot.Signals[0].ValueType != "bool" || snapshot.Signals[0].ChannelIndex != nil {
		t.Fatalf("signals = %#v", snapshot.Signals)
	}
}

func TestDecodeDescriptorSnapshotSchemaTwoModelID(t *testing.T) {
	payload := descriptorFixture(t)
	snapshot, err := DecodeDescriptorSnapshot(payload)
	if err != nil {
		t.Fatal(err)
	}
	if snapshot.SchemaVersion != 2 || len(snapshot.Devices) != 1 ||
		snapshot.Devices[0].ModelID == nil ||
		*snapshot.Devices[0].ModelID != "mcp9600" {
		t.Fatalf("snapshot = %#v", snapshot)
	}

	var unsupportedSchemaOne map[string]any
	if err := json.Unmarshal(payload, &unsupportedSchemaOne); err != nil {
		t.Fatal(err)
	}
	unsupportedSchemaOne["schema_version"] = float64(1)
	delete(unsupportedSchemaOne["devices"].([]any)[0].(map[string]any), "model_id")
	encoded, err := json.Marshal(unsupportedSchemaOne)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := DecodeDescriptorSnapshot(encoded); err == nil {
		t.Fatal("schema 1 descriptor decoded")
	}
}

func TestDecodeDescriptorSnapshotRejectsInvalidModelIDs(t *testing.T) {
	payload := descriptorFixture(t)
	for _, invalid := range []string{
		"", "MCP9600", "-mcp9600", "vendor//model", "model id", "model-",
	} {
		t.Run(invalid, func(t *testing.T) {
			var value map[string]any
			if err := json.Unmarshal(payload, &value); err != nil {
				t.Fatal(err)
			}
			value["devices"].([]any)[0].(map[string]any)["model_id"] = invalid
			encoded, err := json.Marshal(value)
			if err != nil {
				t.Fatal(err)
			}
			if _, err := DecodeDescriptorSnapshot(encoded); err == nil {
				t.Fatalf("invalid model_id %q decoded", invalid)
			}
		})
	}
}

func TestDecodeDescriptorSnapshotRejectsUnknownAndInconsistentContent(t *testing.T) {
	payload := descriptorFixture(t)
	var value map[string]any
	if err := json.Unmarshal(payload, &value); err != nil {
		t.Fatal(err)
	}
	value["provider_payload"] = map[string]any{"secret": true}
	unknown, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := DecodeDescriptorSnapshot(unknown); err == nil {
		t.Fatal("descriptor with unknown field decoded")
	}

	var inconsistent map[string]any
	if err := json.Unmarshal(payload, &inconsistent); err != nil {
		t.Fatal(err)
	}
	inconsistent["signals"].([]any)[0].(map[string]any)["series_key"] = "wrong"
	badSeries, err := json.Marshal(inconsistent)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := DecodeDescriptorSnapshot(badSeries); err == nil {
		t.Fatal("descriptor with inconsistent series_key decoded")
	}
}

func descriptorFixture(t *testing.T) []byte {
	t.Helper()
	payload, err := os.ReadFile(filepath.Join(
		"..", "..", "..", "testdata", "egress", "v2", "descriptor-snapshot.json",
	))
	if err != nil {
		t.Fatal(err)
	}
	return payload
}

func TestDecodeDescriptorSnapshotRejectsOversizeBeforeJSONDecode(t *testing.T) {
	payload := bytes.Repeat([]byte{'x'}, MaxDescriptorBytes+1)
	if _, err := DecodeDescriptorSnapshot(payload); err == nil {
		t.Fatal("oversize descriptor decoded")
	}
}

func TestDecodeDescriptorSnapshotRejectsMalformedIdentityStateAndDuplicates(t *testing.T) {
	payload := descriptorFixture(t)
	mutations := map[string]func(map[string]any){
		"malformed identity": func(value map[string]any) {
			value["devices"].([]any)[0].(map[string]any)["system_id"] = "not-a-uuid"
		},
		"invalid state": func(value map[string]any) {
			value["devices"].([]any)[0].(map[string]any)["state"] = "online"
		},
		"duplicate device": func(value map[string]any) {
			devices := value["devices"].([]any)
			value["devices"] = append(devices, devices[0])
		},
		"duplicate signal": func(value map[string]any) {
			signals := value["signals"].([]any)
			value["signals"] = append(signals, signals[0])
		},
	}
	for name, mutate := range mutations {
		t.Run(name, func(t *testing.T) {
			var value map[string]any
			if err := json.Unmarshal(payload, &value); err != nil {
				t.Fatal(err)
			}
			mutate(value)
			encoded, err := json.Marshal(value)
			if err != nil {
				t.Fatal(err)
			}
			if _, err := DecodeDescriptorSnapshot(encoded); err == nil {
				t.Fatal("malformed descriptor decoded")
			}
		})
	}
}

func TestParseSeriesKeyAcceptsOnlyCanonicalIdentity(t *testing.T) {
	valid := "018f0000-0000-7000-8000-000000000001:temperature:2:primary"
	identity, err := ParseSeriesKey(valid)
	if err != nil {
		t.Fatal(err)
	}
	if identity.SystemID != "018f0000-0000-7000-8000-000000000001" ||
		identity.MeasurementKey != "temperature" || identity.ChannelIndex == nil ||
		*identity.ChannelIndex != 2 || identity.Variant != "primary" {
		t.Fatalf("identity = %#v", identity)
	}
	for _, invalid := range []string{
		"not-a-uuid:temperature:na:primary",
		"018f0000-0000-7000-8000-000000000001::na:primary",
		"018f0000-0000-7000-8000-000000000001:temperature:02:primary",
		"018f0000-0000-7000-8000-000000000001:temperature:-1:primary",
		"018f0000-0000-7000-8000-000000000001:temperature:na:",
		"018f0000-0000-7000-8000-000000000001:Temperature:na:primary",
		"018f0000-0000-7000-8000-000000000001:a..b:na:primary",
		"018f0000-0000-7000-8000-000000000001:" + strings.Repeat("a", 65) + ":na:primary",
	} {
		if _, err := ParseSeriesKey(invalid); err == nil {
			t.Fatalf("non-canonical series key %q was accepted", invalid)
		}
	}
}
