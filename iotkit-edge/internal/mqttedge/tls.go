package mqttedge

import (
	"crypto/tls"
	"crypto/x509"
	"errors"
	"fmt"
	"os"
)

type TrustMode string

const (
	TrustSystemRoots TrustMode = "system_roots"
	TrustBundleOnly  TrustMode = "bundle_only"
)

func LoadTLSConfig(mode TrustMode, bundlePath string) (*tls.Config, error) {
	switch mode {
	case TrustSystemRoots:
		if bundlePath != "" {
			return nil, errors.New("system_roots trust mode does not accept a CA file")
		}
		return &tls.Config{MinVersion: tls.VersionTLS12}, nil
	case TrustBundleOnly:
		if bundlePath == "" {
			return nil, errors.New("bundle_only trust mode requires a CA file")
		}
		contents, err := os.ReadFile(bundlePath)
		if err != nil {
			return nil, fmt.Errorf("read MQTT CA bundle: %w", err)
		}
		roots := x509.NewCertPool()
		if !roots.AppendCertsFromPEM(contents) {
			return nil, errors.New("MQTT CA bundle contains no certificates")
		}
		return &tls.Config{MinVersion: tls.VersionTLS12, RootCAs: roots}, nil
	default:
		return nil, fmt.Errorf("unsupported MQTT trust mode %q", mode)
	}
}
