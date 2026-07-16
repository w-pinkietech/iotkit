package main

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantic"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/store"
)

func TestRunUsageNamesIoTKitSite(t *testing.T) {
	err := run(nil)
	if err == nil || !strings.Contains(err.Error(), "usage: iotkit-site ") {
		t.Fatalf("run error = %v, want iotkit-site usage", err)
	}
	if strings.Contains(err.Error(), "iotkit-site-server") {
		t.Fatalf("run error retains old binary name: %v", err)
	}
}

func TestRunServeRequiresExplicitTLSMode(t *testing.T) {
	dir := t.TempDir()
	passwordFile := filepath.Join(dir, "password")
	if err := os.WriteFile(passwordFile, []byte("test-password\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	dbPath := filepath.Join(dir, "site.db")
	err := run([]string{"serve", "--db", dbPath, "--password-file", passwordFile})
	if err == nil || !strings.Contains(err.Error(), "--trust-mode") {
		t.Fatalf("run serve error = %v", err)
	}
	assertPathDoesNotExist(t, dbPath)
}

func TestRunServeRejectsInvalidTLSModeCombinations(t *testing.T) {
	dir := t.TempDir()
	passwordFile := filepath.Join(dir, "password")
	if err := os.WriteFile(passwordFile, []byte("test-password\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	tests := []struct {
		name        string
		args        []string
		errorSubstr string
	}{
		{name: "unknown", args: []string{"--trust-mode", "automatic"}, errorSubstr: "unsupported MQTT trust mode"},
		{name: "system roots with bundle", args: []string{"--trust-mode", "system_roots", "--ca-file", "ca.pem"}, errorSubstr: "does not accept"},
		{name: "bundle without file", args: []string{"--trust-mode", "bundle_only"}, errorSubstr: "requires a CA file"},
		{name: "plaintext with trust mode", args: []string{"--allow-insecure", "--trust-mode", "system_roots"}, errorSubstr: "cannot be combined"},
		{name: "plaintext with CA file", args: []string{"--allow-insecure", "--ca-file", "ca.pem"}, errorSubstr: "cannot be combined"},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			dbPath := filepath.Join(t.TempDir(), "site.db")
			args := []string{"serve", "--db", dbPath, "--password-file", passwordFile}
			err := run(append(args, test.args...))
			if err == nil {
				t.Fatal("invalid TLS configuration was accepted")
			}
			if !strings.Contains(err.Error(), test.errorSubstr) {
				t.Fatalf("run serve error = %v, want %q", err, test.errorSubstr)
			}
			assertPathDoesNotExist(t, dbPath)
		})
	}
}

func TestMappingSetRequiresExplicitTriggerAndActiveValue(t *testing.T) {
	dbPath := filepath.Join(t.TempDir(), "site.db")
	base := []string{
		"mapping-set", "--db", dbPath,
		"--edge-node-id", "edge-node-01",
		"--series-key", "contact-series-01",
		"--meaning", "production_pulse",
	}

	err := run(append(append([]string{}, base...), "--active-value", "1"))
	if err == nil || !strings.Contains(err.Error(), "--trigger-mode") {
		t.Fatalf("mapping-set missing trigger-mode error = %v", err)
	}

	err = run(append(append([]string{}, base...), "--trigger-mode", "active_sample"))
	if err == nil || !strings.Contains(err.Error(), "--active-value") {
		t.Fatalf("mapping-set missing active-value error = %v", err)
	}
}

func TestMappingSetAcceptsExplicitZeroActiveValue(t *testing.T) {
	dbPath := filepath.Join(t.TempDir(), "site.db")
	if err := run([]string{
		"mapping-set", "--db", dbPath,
		"--edge-node-id", "edge-node-01",
		"--series-key", "contact-series-01",
		"--meaning", "production_pulse",
		"--trigger-mode", "active_edge",
		"--active-value", "0",
	}); err != nil {
		t.Fatal(err)
	}

	archive, err := store.Open(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = archive.Close() })
	mappings, err := archive.ListSemanticMappings(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(mappings) != 1 || mappings[0].ActiveValue != 0 {
		t.Fatalf("mappings = %+v", mappings)
	}
}

func TestMappingSetRouteAddAndDeactivateUseAuditedApplicationService(t *testing.T) {
	dbPath := filepath.Join(t.TempDir(), "site.db")
	if err := run([]string{
		"mapping-set", "--db", dbPath,
		"--edge-node-id", "edge-node-01",
		"--series-key", "contact-series-01",
		"--meaning", "production_pulse",
		"--trigger-mode", "active_edge",
		"--active-value", "1",
	}); err != nil {
		t.Fatal(err)
	}

	archive, err := store.Open(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	mappings, err := archive.ListSemanticMappings(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if err := archive.Close(); err != nil {
		t.Fatal(err)
	}
	if len(mappings) != 1 {
		t.Fatalf("mappings = %#v", mappings)
	}
	if err := run([]string{
		"route-add", "--db", dbPath,
		"--mapping-id", mappings[0].ID,
		"--topic", "factory/production-pulses",
	}); err != nil {
		t.Fatal(err)
	}
	if err := run([]string{
		"mapping-deactivate", "--db", dbPath,
		"--edge-node-id", "edge-node-01",
		"--series-key", "contact-series-01",
	}); err != nil {
		t.Fatal(err)
	}

	archive, err = store.Open(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = archive.Close() })
	events, err := archive.ListAuditEvents(context.Background(), 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(events) != 3 {
		t.Fatalf("audit events = %d, want 3", len(events))
	}
	if events[0].Operation != "semantic_mapping.deactivate" ||
		events[1].Operation != "legacy_mqtt_route.put" ||
		events[2].Operation != "semantic_mapping.put" {
		t.Fatalf("audit events = %#v", events)
	}
}

func TestMappingSetRejectsInvalidSpecBeforeCreatingDatabase(t *testing.T) {
	tests := []struct {
		name  string
		flags []string
	}{
		{name: "active value", flags: []string{"--meaning", "production_pulse", "--trigger-mode", "active_sample", "--active-value", "2"}},
		{name: "meaning", flags: []string{"--meaning", "unsupported", "--trigger-mode", "active_sample", "--active-value", "1"}},
		{name: "trigger mode", flags: []string{"--meaning", "production_pulse", "--trigger-mode", "unsupported", "--active-value", "1"}},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			dbPath := filepath.Join(t.TempDir(), "site.db")
			args := []string{
				"mapping-set", "--db", dbPath,
				"--edge-node-id", "edge-node-01",
				"--series-key", "contact-series-01",
			}
			if err := run(append(args, test.flags...)); err == nil {
				t.Fatal("mapping-set accepted invalid semantic mapping")
			}
			assertPathDoesNotExist(t, dbPath)
		})
	}
}

func TestRouteAddRejectsInvalidTopicBeforeCreatingDatabase(t *testing.T) {
	dbPath := filepath.Join(t.TempDir(), "site.db")
	if err := run([]string{
		"route-add", "--db", dbPath,
		"--mapping-id", "sm-not-created",
		"--topic", "/invalid/topic",
	}); err == nil {
		t.Fatal("route-add accepted invalid MQTT topic")
	}
	assertPathDoesNotExist(t, dbPath)
}

func TestRouteListWritesDeliveryStatusJSON(t *testing.T) {
	dbPath := filepath.Join(t.TempDir(), "site.db")
	archive, err := store.Open(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	mapping, err := archive.ApplySemanticMapping(context.Background(), siteapp.LocalCLIActor(), semantic.MappingSpec{
		EdgeNodeID:  "edge-node-01",
		SeriesKey:   "contact-series-01",
		Meaning:     semantic.MeaningProductionPulse,
		TriggerMode: semantic.TriggerActiveEdge,
		ActiveValue: 1,
	}, siteapp.RevisionPrecondition{})
	if err != nil {
		t.Fatal(err)
	}
	route, err := archive.ApplyLegacyMQTTRoute(
		context.Background(), siteapp.LocalCLIActor(), mapping.ID, "factory/production-pulses",
	)
	if err != nil {
		t.Fatal(err)
	}
	if err := archive.Close(); err != nil {
		t.Fatal(err)
	}

	outputPath := filepath.Join(t.TempDir(), "route-list.json")
	output, err := os.Create(outputPath)
	if err != nil {
		t.Fatal(err)
	}
	originalStdout := os.Stdout
	os.Stdout = output
	t.Cleanup(func() { os.Stdout = originalStdout })
	if err := run([]string{"route-list", "--db", dbPath}); err != nil {
		t.Fatal(err)
	}
	os.Stdout = originalStdout
	if err := output.Close(); err != nil {
		t.Fatal(err)
	}

	encoded, err := os.ReadFile(outputPath)
	if err != nil {
		t.Fatal(err)
	}
	var statuses []store.MQTTRouteStatus
	if err := json.Unmarshal(encoded, &statuses); err != nil {
		t.Fatalf("decode route-list output %s: %v", encoded, err)
	}
	if len(statuses) != 1 || statuses[0].RouteID != route.RouteID ||
		statuses[0].MappingID != mapping.ID || statuses[0].PendingCount != 0 ||
		statuses[0].PublishedCount != 0 || statuses[0].OldestPendingAt != nil {
		t.Fatalf("statuses = %#v", statuses)
	}
}

func assertPathDoesNotExist(t *testing.T, path string) {
	t.Helper()
	if _, err := os.Stat(path); err == nil {
		t.Fatalf("invalid command created %s", path)
	} else if !os.IsNotExist(err) {
		t.Fatalf("stat %s: %v", path, err)
	}
}
