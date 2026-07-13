package main

import (
	"crypto/tls"
	"os"
	"path/filepath"
	"testing"
)

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
