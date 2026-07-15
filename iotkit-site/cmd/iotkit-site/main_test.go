package main

import (
	"context"
	"crypto/tls"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantic"
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

func TestLoadTLSConfigUsesSystemRootsWhenCAFileIsOmitted(t *testing.T) {
	config, err := loadTLSConfig("")
	if err != nil {
		t.Fatal(err)
	}
	if config == nil || config.MinVersion != tls.VersionTLS12 {
		t.Fatalf("unexpected TLS config: %#v", config)
	}
	if config.RootCAs != nil {
		t.Fatal("system root selection should remain delegated to crypto/tls")
	}
}

func TestLoadTLSConfigRejectsFileWithoutCertificates(t *testing.T) {
	path := filepath.Join(t.TempDir(), "not-a-ca.pem")
	if err := os.WriteFile(path, []byte("not a certificate"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := loadTLSConfig(path); err == nil {
		t.Fatal("invalid CA file was accepted")
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
	mapping, err := archive.PutSemanticMapping(context.Background(), semantic.MappingSpec{
		EdgeNodeID:  "edge-node-01",
		SeriesKey:   "contact-series-01",
		Meaning:     semantic.MeaningProductionPulse,
		TriggerMode: semantic.TriggerActiveEdge,
		ActiveValue: 1,
	})
	if err != nil {
		t.Fatal(err)
	}
	route, err := archive.PutMQTTRoute(context.Background(), mapping.ID, "factory/production-pulses")
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
