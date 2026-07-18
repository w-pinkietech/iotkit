package contract

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func validActivationRequest() ActivationRequest {
	return ActivationRequest{
		SchemaVersion:       1,
		ActivationID:        "act-0123456789abcdef0123456789abcdef",
		SiteID:              "site-0123456789abcdef0123456789abcdef",
		EdgeNodeID:          "edge-node-01",
		ExpectedLedgerEpoch: "epoch-01",
		GrantRevision:       1,
		IssuedAt:            10,
	}
}

func validActivationResult() ActivationResult {
	return ActivationResult{
		SchemaVersion:            1,
		ActivationID:             "act-0123456789abcdef0123456789abcdef",
		SiteID:                   "site-0123456789abcdef0123456789abcdef",
		EdgeNodeID:               "edge-node-01",
		LedgerEpoch:              "epoch-01",
		Status:                   "applied",
		DiscardThroughReadingSeq: 12,
		FirstPublicationSeq:      1,
		AppliedAt:                20,
	}
}

func TestActivationRequestRoundTripsStrictly(t *testing.T) {
	request := validActivationRequest()
	payload, err := request.Encode()
	if err != nil {
		t.Fatal(err)
	}
	decoded, err := DecodeActivationRequest(payload)
	if err != nil {
		t.Fatal(err)
	}
	if decoded != request {
		t.Fatalf("decoded = %#v, want %#v", decoded, request)
	}

	var object map[string]any
	if err := json.Unmarshal(payload, &object); err != nil {
		t.Fatal(err)
	}
	object["unknown"] = true
	unknown, err := json.Marshal(object)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := DecodeActivationRequest(unknown); err == nil {
		t.Fatal("unknown activation request field was accepted")
	}
}

func TestActivationContractsRejectMalformedIdentityVersionAndBoundary(t *testing.T) {
	request := validActivationRequest()
	request.ActivationID = "act-0123456789ABCDEF0123456789ABCDEF"
	if _, err := request.Encode(); err == nil {
		t.Fatal("uppercase activation ID was accepted")
	}
	request = validActivationRequest()
	request.SiteID = "site-short"
	if _, err := request.Encode(); err == nil {
		t.Fatal("short Site ID was accepted")
	}
	request = validActivationRequest()
	request.EdgeNodeID = "edge/other"
	if _, err := request.Encode(); err == nil {
		t.Fatal("unsafe Edge ID was accepted")
	}
	request = validActivationRequest()
	request.GrantRevision = 2
	if _, err := request.Encode(); err == nil {
		t.Fatal("unsupported grant revision was accepted")
	}

	result := validActivationResult()
	result.FirstPublicationSeq = 2
	payload, _ := json.Marshal(result)
	if _, err := DecodeActivationResult(payload); err == nil {
		t.Fatal("activation result starting after pub_seq 1 was accepted")
	}
	result = validActivationResult()
	result.DiscardThroughReadingSeq = -1
	payload, _ = json.Marshal(result)
	if _, err := DecodeActivationResult(payload); err == nil {
		t.Fatal("negative discard boundary was accepted")
	}
}

func TestActivationResultEncodingIsStableForRetry(t *testing.T) {
	result := validActivationResult()
	first, err := result.Encode()
	if err != nil {
		t.Fatal(err)
	}
	decoded, err := DecodeActivationResult(first)
	if err != nil {
		t.Fatal(err)
	}
	second, err := decoded.Encode()
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(first, second) {
		t.Fatalf("activation result encoding changed: %s != %s", first, second)
	}
}

func TestSharedActivationFixturesMatchTheSiteContract(t *testing.T) {
	fixture := func(name string) []byte {
		t.Helper()
		payload, err := os.ReadFile(filepath.Join(
			"..", "..", "..", "testdata", "egress", "v1", name,
		))
		if err != nil {
			t.Fatal(err)
		}
		return payload
	}
	request, err := DecodeActivationRequest(fixture("activation-request.json"))
	if err != nil {
		t.Fatal(err)
	}
	result, err := DecodeActivationResult(fixture("activation-result.json"))
	if err != nil {
		t.Fatal(err)
	}
	if request.EdgeNodeID != result.EdgeNodeID ||
		request.ExpectedLedgerEpoch != result.LedgerEpoch {
		t.Fatalf("request/result identity mismatch: %#v %#v", request, result)
	}
	for _, name := range []string{
		"activation-request-malformed-id.json",
		"activation-request-unknown-field.json",
	} {
		if _, err := DecodeActivationRequest(fixture(name)); err == nil {
			t.Fatalf("%s was accepted", name)
		}
	}
	if _, err := DecodeActivationResult(
		fixture("activation-result-first-seq-2.json"),
	); err == nil {
		t.Fatal("activation result beginning at pub_seq 2 was accepted")
	}
	for _, name := range []string{
		"activation-request-wrong-edge.json",
		"activation-request-wrong-epoch.json",
		"activation-request-conflicting-id.json",
	} {
		if _, err := DecodeActivationRequest(fixture(name)); err != nil {
			t.Fatalf("%s must be schema-valid for contextual rejection: %v", name, err)
		}
	}
}
