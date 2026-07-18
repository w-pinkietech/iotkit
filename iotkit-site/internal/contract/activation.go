package contract

import (
	"encoding/json"
	"fmt"
	"strings"
	"unicode"
)

type ActivationRequest struct {
	SchemaVersion       uint32 `json:"schema_version"`
	ActivationID        string `json:"activation_id"`
	SiteID              string `json:"site_id"`
	EdgeNodeID          string `json:"edge_node_id"`
	ExpectedLedgerEpoch string `json:"expected_ledger_epoch"`
	GrantRevision       uint64 `json:"grant_revision"`
	IssuedAt            int64  `json:"issued_at"`
}

type ActivationResult struct {
	SchemaVersion            uint32 `json:"schema_version"`
	ActivationID             string `json:"activation_id"`
	SiteID                   string `json:"site_id"`
	EdgeNodeID               string `json:"edge_node_id"`
	LedgerEpoch              string `json:"ledger_epoch"`
	Status                   string `json:"status"`
	DiscardThroughReadingSeq int64  `json:"discard_through_reading_seq"`
	FirstPublicationSeq      int64  `json:"first_publication_seq"`
	AppliedAt                int64  `json:"applied_at"`
}

func DecodeActivationRequest(payload []byte) (ActivationRequest, error) {
	var request ActivationRequest
	if err := decodeOneStrict(payload, &request); err != nil {
		return request, fmt.Errorf("decode activation request: %w", err)
	}
	if err := request.Validate(); err != nil {
		return request, err
	}
	return request, nil
}

func DecodeActivationResult(payload []byte) (ActivationResult, error) {
	var result ActivationResult
	if err := decodeOneStrict(payload, &result); err != nil {
		return result, fmt.Errorf("decode activation result: %w", err)
	}
	if err := result.Validate(); err != nil {
		return result, err
	}
	return result, nil
}

func (request ActivationRequest) Validate() error {
	if request.SchemaVersion != SchemaVersion {
		return invalid("activation request schema_version must be 1")
	}
	if err := validatePrefixedHex("activation_id", request.ActivationID, "act-"); err != nil {
		return err
	}
	if err := validatePrefixedHex("site_id", request.SiteID, "site-"); err != nil {
		return err
	}
	if err := validateActivationTopicSegment("edge_node_id", request.EdgeNodeID); err != nil {
		return err
	}
	if err := validateActivationIdentity("expected_ledger_epoch", request.ExpectedLedgerEpoch); err != nil {
		return err
	}
	if request.GrantRevision != 1 {
		return invalid("activation grant_revision must be 1")
	}
	if request.IssuedAt < 0 {
		return invalid("activation issued_at must be non-negative")
	}
	return nil
}

func (result ActivationResult) Validate() error {
	if result.SchemaVersion != SchemaVersion {
		return invalid("activation result schema_version must be 1")
	}
	if err := validatePrefixedHex("activation_id", result.ActivationID, "act-"); err != nil {
		return err
	}
	if err := validatePrefixedHex("site_id", result.SiteID, "site-"); err != nil {
		return err
	}
	if err := validateActivationTopicSegment("edge_node_id", result.EdgeNodeID); err != nil {
		return err
	}
	if err := validateActivationIdentity("ledger_epoch", result.LedgerEpoch); err != nil {
		return err
	}
	if result.Status != "applied" {
		return invalid("activation result status must be applied")
	}
	if result.DiscardThroughReadingSeq < 0 {
		return invalid("discard boundary must be non-negative")
	}
	if result.FirstPublicationSeq != 1 {
		return invalid("first_publication_seq must be 1")
	}
	if result.AppliedAt < 0 {
		return invalid("activation applied_at must be non-negative")
	}
	return nil
}

func (request ActivationRequest) Encode() ([]byte, error) {
	if err := request.Validate(); err != nil {
		return nil, err
	}
	return json.Marshal(request)
}

func (result ActivationResult) Encode() ([]byte, error) {
	if err := result.Validate(); err != nil {
		return nil, err
	}
	return json.Marshal(result)
}

func (request ActivationRequest) ValidateTopicEdge(edgeNodeID string) error {
	if request.EdgeNodeID != edgeNodeID {
		return invalid("activation request topic/body edge_node_id mismatch")
	}
	return nil
}

func (result ActivationResult) ValidateTopicEdge(edgeNodeID string) error {
	if result.EdgeNodeID != edgeNodeID {
		return invalid("activation result topic/body edge_node_id mismatch")
	}
	return nil
}

func validatePrefixedHex(field, value, prefix string) error {
	random, found := strings.CutPrefix(value, prefix)
	if !found {
		return invalid(field + " has an invalid prefix")
	}
	if len(random) != 32 {
		return invalid(field + " must contain 128-bit lowercase hexadecimal")
	}
	for _, character := range random {
		if !strings.ContainsRune("0123456789abcdef", character) {
			return invalid(field + " must contain 128-bit lowercase hexadecimal")
		}
	}
	return nil
}

func validateActivationTopicSegment(field, value string) error {
	if err := validateActivationIdentity(field, value); err != nil {
		return err
	}
	if strings.ContainsAny(value, "/+#") {
		return invalid(field + " is not a safe MQTT topic segment")
	}
	return nil
}

func validateActivationIdentity(field, value string) error {
	if value == "" || len(value) > 255 || strings.Contains(value, ":") ||
		strings.IndexFunc(value, unicode.IsControl) >= 0 {
		return invalid(field + " is not a valid identity")
	}
	return nil
}
