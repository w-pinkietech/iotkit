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
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantic"
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
		return errors.New("usage: iotkit-site <serve|query|mapping-set|mapping-list|route-add|route-list|semantic-query> [options]")
	}
	switch args[0] {
	case "serve":
		return runServe(args[1:])
	case "query":
		return runQuery(args[1:])
	case "mapping-set":
		return runMappingSet(args[1:])
	case "mapping-list":
		return runMappingList(args[1:])
	case "route-add":
		return runRouteAdd(args[1:])
	case "route-list":
		return runRouteList(args[1:])
	case "semantic-query":
		return runSemanticQuery(args[1:])
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
	}, mqttsite.Processor{Store: archive}, archive, slog.Default())
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
	return writeJSON(records)
}

func runMappingSet(args []string) error {
	flags := flag.NewFlagSet("mapping-set", flag.ContinueOnError)
	dbPath := flags.String("db", "site.db", "Site SQLite path")
	edgeNodeID := flags.String("edge-node-id", "", "source Edge Node ID")
	seriesKey := flags.String("series-key", "", "source series key")
	meaning := flags.String("meaning", "", "semantic meaning")
	triggerMode := flags.String("trigger-mode", "", "trigger mode")
	activeValue := flags.Int("active-value", -1, "contact value considered active (0 or 1)")
	if err := flags.Parse(args); err != nil {
		return err
	}
	if *edgeNodeID == "" {
		return errors.New("--edge-node-id is required")
	}
	if *seriesKey == "" {
		return errors.New("--series-key is required")
	}
	if *meaning == "" {
		return errors.New("--meaning is required")
	}
	if *triggerMode == "" {
		return errors.New("--trigger-mode is required")
	}
	if *activeValue == -1 {
		return errors.New("--active-value is required")
	}
	spec := semantic.MappingSpec{
		EdgeNodeID:  *edgeNodeID,
		SeriesKey:   *seriesKey,
		Meaning:     semantic.Meaning(*meaning),
		TriggerMode: semantic.TriggerMode(*triggerMode),
		ActiveValue: *activeValue,
	}
	if err := spec.Validate(); err != nil {
		return err
	}

	archive, err := store.Open(*dbPath)
	if err != nil {
		return err
	}
	defer archive.Close()
	mapping, err := archive.PutSemanticMapping(context.Background(), spec)
	if err != nil {
		return err
	}
	return writeJSON(mapping)
}

func runMappingList(args []string) error {
	flags := flag.NewFlagSet("mapping-list", flag.ContinueOnError)
	dbPath := flags.String("db", "site.db", "Site SQLite path")
	if err := flags.Parse(args); err != nil {
		return err
	}
	archive, err := store.Open(*dbPath)
	if err != nil {
		return err
	}
	defer archive.Close()
	mappings, err := archive.ListSemanticMappings(context.Background())
	if err != nil {
		return err
	}
	return writeJSON(mappings)
}

func runRouteAdd(args []string) error {
	flags := flag.NewFlagSet("route-add", flag.ContinueOnError)
	dbPath := flags.String("db", "site.db", "Site SQLite path")
	mappingID := flags.String("mapping-id", "", "semantic mapping ID")
	topic := flags.String("topic", "", "application MQTT topic")
	if err := flags.Parse(args); err != nil {
		return err
	}
	if *mappingID == "" {
		return errors.New("--mapping-id is required")
	}
	if *topic == "" {
		return errors.New("--topic is required")
	}
	spec := store.MQTTRouteSpec{MappingID: *mappingID, Topic: *topic}
	if err := spec.Validate(); err != nil {
		return err
	}
	archive, err := store.Open(*dbPath)
	if err != nil {
		return err
	}
	defer archive.Close()
	route, err := archive.PutMQTTRoute(context.Background(), *mappingID, *topic)
	if err != nil {
		return err
	}
	return writeJSON(route)
}

func runRouteList(args []string) error {
	flags := flag.NewFlagSet("route-list", flag.ContinueOnError)
	dbPath := flags.String("db", "site.db", "Site SQLite path")
	if err := flags.Parse(args); err != nil {
		return err
	}
	archive, err := store.Open(*dbPath)
	if err != nil {
		return err
	}
	defer archive.Close()
	statuses, err := archive.ListMQTTRouteStatuses(context.Background())
	if err != nil {
		return err
	}
	return writeJSON(statuses)
}

func runSemanticQuery(args []string) error {
	flags := flag.NewFlagSet("semantic-query", flag.ContinueOnError)
	dbPath := flags.String("db", "site.db", "Site SQLite path")
	limit := flags.Int("limit", 100, "maximum semantic events")
	if err := flags.Parse(args); err != nil {
		return err
	}
	archive, err := store.Open(*dbPath)
	if err != nil {
		return err
	}
	defer archive.Close()
	events, err := archive.ListSemanticEvents(context.Background(), *limit)
	if err != nil {
		return err
	}
	return writeJSON(events)
}

func writeJSON(value any) error {
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	return encoder.Encode(value)
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
