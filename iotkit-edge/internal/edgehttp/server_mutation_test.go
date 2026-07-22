package edgehttp

import (
	"errors"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
)

func TestConsoleMutationResultReturnsToTheEditedSection(t *testing.T) {
	request := httptest.NewRequest(
		http.MethodPost,
		"/console/semantic-rules/rule_01",
		strings.NewReader(url.Values{
			"return_anchor": {"rule-rule_01"},
		}.Encode()),
	)
	request.Header.Set("Content-Type", "application/x-www-form-urlencoded")

	response := httptest.NewRecorder()
	(&Server{}).consoleMutationResult(
		response,
		request,
		"/sensors/sig_0123456789abcdef0123456789abcdef",
		nil,
	)

	if response.Code != http.StatusSeeOther {
		t.Fatalf("status = %d, want 303", response.Code)
	}
	if location := response.Header().Get("Location"); location !=
		"/sensors/sig_0123456789abcdef0123456789abcdef?focus=rule-rule_01&saved=1#rule-rule_01" {
		t.Fatalf("Location = %q", location)
	}
}

func TestConsoleMutationResultExplainsThresholdOrderErrors(t *testing.T) {
	request := httptest.NewRequest(
		http.MethodPost,
		"/console/semantic-rules/rule_01",
		strings.NewReader(url.Values{
			"return_anchor": {"rule-rule_01"},
		}.Encode()),
	)
	request.Header.Set("Content-Type", "application/x-www-form-urlencoded")

	response := httptest.NewRecorder()
	(&Server{}).consoleMutationResult(
		response,
		request,
		"/sensors/sig_0123456789abcdef0123456789abcdef",
		errors.New("semantic falling threshold cannot exceed rising threshold"),
	)

	if location := response.Header().Get("Location"); location !=
		"/sensors/sig_0123456789abcdef0123456789abcdef?error=threshold_order&focus=rule-rule_01#rule-rule_01" {
		t.Fatalf("Location = %q", location)
	}
}

const testOrigin = "https://iotkit.example.test"
