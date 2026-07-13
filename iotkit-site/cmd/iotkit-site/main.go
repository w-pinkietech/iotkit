package main

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"log/slog"
	"os"
	"os/signal"
	"strings"
	"syscall"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/mqttsite"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/store"
)

func main() {
	if err := run(os.Args[1:]); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func run(args []string) error {
	if len(args) == 0 {
		return errors.New("usage: iotkit-site <serve|query> [options]")
	}
	switch args[0] {
	case "serve":
		return runServe(args[1:])
	case "query":
		return runQuery(args[1:])
	default:
		return fmt.Errorf("unknown command %q", args[0])
	}
}

func runServe(args []string) error {
	flags := flag.NewFlagSet("serve", flag.ContinueOnError)
	dbPath := flags.String("db", "site.db", "Site SQLite path")
	brokerURL := flags.String("broker-url", "", "MQTT broker URL")
	clientID := flags.String("client-id", "iotkit-site", "MQTT client ID")
	username := flags.String("username", "", "MQTT username")
	passwordFile := flags.String("password-file", "", "file containing MQTT password")
	caFile := flags.String("ca-file", "", "optional PEM CA file")
	allowInsecure := flags.Bool("allow-insecure", false, "allow plain MQTT for local tests")
	if err := flags.Parse(args); err != nil {
		return err
	}
	if *passwordFile == "" {
		return errors.New("--password-file is required")
	}
	passwordBytes, err := os.ReadFile(*passwordFile)
	if err != nil {
		return fmt.Errorf("read MQTT password file: %w", err)
	}
	password := strings.TrimRight(string(passwordBytes), "\r\n")
	if password == "" {
		return errors.New("MQTT password file is empty")
	}
	tlsConfig, err := loadTLSConfig(*caFile)
	if err != nil {
		return err
	}

	archive, err := store.Open(*dbPath)
	if err != nil {
		return fmt.Errorf("open Site store: %w", err)
	}
	defer archive.Close()
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	return mqttsite.Run(ctx, mqttsite.ClientConfig{
		BrokerURL:     *brokerURL,
		ClientID:      *clientID,
		Username:      *username,
		Password:      password,
		TLSConfig:     tlsConfig,
		AllowInsecure: *allowInsecure,
	}, mqttsite.Processor{Store: archive}, slog.Default())
}

func runQuery(args []string) error {
	flags := flag.NewFlagSet("query", flag.ContinueOnError)
	dbPath := flags.String("db", "site.db", "Site SQLite path")
	limit := flags.Int("limit", 100, "maximum raw records")
	if err := flags.Parse(args); err != nil {
		return err
	}
	archive, err := store.Open(*dbPath)
	if err != nil {
		return err
	}
	defer archive.Close()
	records, err := archive.ListRawRecords(context.Background(), *limit)
	if err != nil {
		return err
	}
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	return encoder.Encode(records)
}

func loadTLSConfig(caFile string) (*tls.Config, error) {
	if caFile == "" {
		return &tls.Config{MinVersion: tls.VersionTLS12}, nil
	}
	roots, err := x509.SystemCertPool()
	if err != nil {
		return nil, err
	}
	pem, err := os.ReadFile(caFile)
	if err != nil {
		return nil, err
	}
	if !roots.AppendCertsFromPEM(pem) {
		return nil, errors.New("CA file contains no certificates")
	}
	return &tls.Config{MinVersion: tls.VersionTLS12, RootCAs: roots}, nil
}
