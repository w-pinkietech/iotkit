package contract

import (
	"bytes"
	"encoding/json"
	"testing"
)

func TestDecodeDescriptorSnapshotFixture(t *testing.T) {
	snapshot, err := DecodeDescriptorSnapshot(fixture(t, "descriptor-snapshot.json"))
	if err != nil {
		t.Fatal(err)
	}
	if snapshot.EdgeNodeID != "edge-node-01" || snapshot.DescriptorRevision != 4 || !snapshot.Complete {
		t.Fatalf("snapshot = %#v", snapshot)
	}
	if len(snapshot.Devices) != 1 || snapshot.Devices[0].Identifier == nil || *snapshot.Devices[0].Identifier != "01234567" {
		t.Fatalf("devices = %#v", snapshot.Devices)
	}
	if len(snapshot.Signals) != 1 || snapshot.Signals[0].ValueType != "bool" || snapshot.Signals[0].ChannelIndex != nil {
		t.Fatalf("signals = %#v", snapshot.Signals)
	}
}

func TestDecodeDescriptorSnapshotRejectsUnknownAndInconsistentContent(t *testing.T) {
	payload := fixture(t, "descriptor-snapshot.json")
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

func TestDecodeDescriptorSnapshotRejectsOversizeBeforeJSONDecode(t *testing.T) {
	payload := bytes.Repeat([]byte{'x'}, MaxDescriptorBytes+1)
	if _, err := DecodeDescriptorSnapshot(payload); err == nil {
		t.Fatal("oversize descriptor decoded")
	}
}

func TestDecodeDescriptorSnapshotRejectsMalformedIdentityStateAndDuplicates(t *testing.T) {
	payload := fixture(t, "descriptor-snapshot.json")
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
	} {
		if _, err := ParseSeriesKey(invalid); err == nil {
			t.Fatalf("non-canonical series key %q was accepted", invalid)
		}
	}
}
