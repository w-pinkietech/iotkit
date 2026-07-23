package mqttedge

import (
	"crypto/rand"
	"crypto/rsa"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/pem"
	"math/big"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestLoadTLSConfigSystemRoots(t *testing.T) {
	config, err := LoadTLSConfig(TrustSystemRoots, "")
	if err != nil {
		t.Fatal(err)
	}
	if config.MinVersion != tls.VersionTLS12 || config.RootCAs != nil {
		t.Fatalf("config = %#v", config)
	}
}

func TestLoadTLSConfigBundleOnlyDoesNotInheritSystemRoots(t *testing.T) {
	bundle := filepath.Join(t.TempDir(), "ca.pem")
	if err := os.WriteFile(bundle, testRootCertificatePEM(t), 0o600); err != nil {
		t.Fatal(err)
	}
	config, err := LoadTLSConfig(TrustBundleOnly, bundle)
	if err != nil {
		t.Fatal(err)
	}
	if config.RootCAs == nil {
		t.Fatal("bundle-only trust has no root pool")
	}
	if subjects := config.RootCAs.Subjects(); len(subjects) != 1 {
		t.Fatalf("bundle-only subjects = %d, want 1", len(subjects))
	}
}

func TestLoadTLSConfigRejectsInvalidCombinations(t *testing.T) {
	tests := []struct {
		name string
		mode TrustMode
		path string
	}{
		{name: "unknown mode", mode: "automatic"},
		{name: "system roots with bundle", mode: TrustSystemRoots, path: "ca.pem"},
		{name: "bundle without file", mode: TrustBundleOnly},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			if _, err := LoadTLSConfig(test.mode, test.path); err == nil {
				t.Fatal("invalid trust configuration was accepted")
			}
		})
	}
}

func testRootCertificatePEM(t *testing.T) []byte {
	t.Helper()
	key, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		t.Fatal(err)
	}
	now := time.Now()
	template := &x509.Certificate{
		SerialNumber:          big.NewInt(1),
		Subject:               pkix.Name{CommonName: "IoTKit unit-test root"},
		NotBefore:             now.Add(-time.Hour),
		NotAfter:              now.Add(time.Hour),
		IsCA:                  true,
		BasicConstraintsValid: true,
		KeyUsage:              x509.KeyUsageCertSign,
	}
	der, err := x509.CreateCertificate(rand.Reader, template, template, &key.PublicKey, key)
	if err != nil {
		t.Fatal(err)
	}
	return pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der})
}
