package contract

import (
	"bytes"
	"encoding/json"
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
