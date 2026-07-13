package main

import (
	"crypto/tls"
	"os"
	"path/filepath"
	"strings"
	"testing"
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
