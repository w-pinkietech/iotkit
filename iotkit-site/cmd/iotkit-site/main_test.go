package main

import (
	"context"
	"crypto/tls"
	"os"
	"path/filepath"
	"strings"
	"testing"

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
