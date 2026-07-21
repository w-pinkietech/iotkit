package main

import (
	"context"
	"crypto/tls"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"log/slog"
	"net"
	"net/http"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/mqttsite"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantic"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/sitehttp"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/sitesession"
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
		return errors.New("usage: iotkit-site <serve|account|backup|diagnose|query|mapping-set|mapping-deactivate|mapping-list|route-add|route-list|semantic-query> [options]")
	}
	switch args[0] {
	case "serve":
		return runServe(args[1:])
	case "account":
		return runAccount(args[1:])
	case "backup":
		return runBackup(args[1:])
	case "diagnose":
		return runDiagnose(args[1:])
	case "query":
		return runQuery(args[1:])
	case "mapping-set":
		return runMappingSet(args[1:])
	case "mapping-deactivate":
		return runMappingDeactivate(args[1:])
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

func runDiagnose(args []string) error {
	flags := flag.NewFlagSet("diagnose", flag.ContinueOnError)
	dbPath := flags.String("db", "site.db", "Site SQLite path")
	storageWarningPercent := flags.Int("storage-warning-percent", 90, "Site filesystem usage warning percent")
	if err := flags.Parse(args); err != nil {
		return err
	}
	if err := requireExistingRegularFile(*dbPath, "--db must name an existing Site database"); err != nil {
		return err
	}
	archive, err := store.Open(*dbPath)
	if err != nil {
		return err
	}
	defer archive.Close()
	report, err := archive.GetDiagnostics(
		context.Background(), *storageWarningPercent, time.Now(),
	)
	if err != nil {
		return err
	}
	return writeJSON(report)
}

func runBackup(args []string) error {
	if len(args) == 0 {
		return errors.New("usage: iotkit-site backup <create|restore|accept-archive-loss> [options]")
	}
	switch args[0] {
	case "create":
		return runBackupCreate(args[1:])
	case "restore":
		return runBackupRestore(args[1:])
	case "accept-archive-loss":
		return runBackupAcceptArchiveLoss(args[1:])
	default:
		return fmt.Errorf("unknown backup command %q", args[0])
	}
}

func runBackupCreate(args []string) error {
	flags := flag.NewFlagSet("backup create", flag.ContinueOnError)
	dbPath := flags.String("db", "site.db", "existing Site SQLite path")
	output := flags.String("output", "", "new encrypted backup path")
	passphraseFile := flags.String("passphrase-file", "", "owner-only file containing the backup passphrase")
	if err := flags.Parse(args); err != nil {
		return err
	}
	if *output == "" || *passphraseFile == "" {
		return errors.New("--output and --passphrase-file are required")
	}
	if err := requireExistingRegularFile(*dbPath, "--db must name an existing Site database"); err != nil {
		return err
	}
	passphrase, err := readOwnerOnlySecret(*passphraseFile)
	if err != nil {
		return err
	}
	archive, err := store.Open(*dbPath)
	if err != nil {
		return err
	}
	defer archive.Close()
	result, err := siteapp.NewService(archive).Dispatch(
		context.Background(), siteapp.LocalCLIActor(), siteapp.CreateSiteBackup{
			Destination: *output,
			Passphrase:  passphrase,
		},
	)
	if err != nil {
		return err
	}
	return writeJSON(result.Backup)
}

func runBackupRestore(args []string) error {
	flags := flag.NewFlagSet("backup restore", flag.ContinueOnError)
	input := flags.String("input", "", "encrypted backup path")
	dbPath := flags.String("db", "site.db", "new restored Site SQLite path")
	passphraseFile := flags.String("passphrase-file", "", "owner-only file containing the backup passphrase")
	if err := flags.Parse(args); err != nil {
		return err
	}
	if *input == "" || *passphraseFile == "" {
		return errors.New("--input and --passphrase-file are required")
	}
	passphrase, err := readOwnerOnlySecret(*passphraseFile)
	if err != nil {
		return err
	}
	manifest, err := store.RestoreEncryptedBackup(
		context.Background(), *input, *dbPath, passphrase,
	)
	if err != nil {
		return err
	}
	return writeJSON(manifest)
}

func runBackupAcceptArchiveLoss(args []string) error {
	flags := flag.NewFlagSet("backup accept-archive-loss", flag.ContinueOnError)
	dbPath := flags.String("db", "site.db", "Site SQLite path")
	edgeNodeID := flags.String("edge-node-id", "", "Edge node identity in recovery hold")
	ledgerEpoch := flags.String("ledger-epoch", "", "Edge ledger epoch")
	confirmSiteID := flags.String("confirm-site-id", "", "Site ID typed to confirm the destructive decision")
	reason := flags.String("reason", "", "operator reason recorded in the audit log")
	if err := flags.Parse(args); err != nil {
		return err
	}
	if *edgeNodeID == "" || *ledgerEpoch == "" || *confirmSiteID == "" || *reason == "" {
		return errors.New("--edge-node-id, --ledger-epoch, --confirm-site-id, and --reason are required")
	}
	if err := requireExistingRegularFile(*dbPath, "--db must name an existing Site database"); err != nil {
		return err
	}
	archive, err := store.Open(*dbPath)
	if err != nil {
		return err
	}
	defer archive.Close()
	result, err := siteapp.NewService(archive).Dispatch(
		context.Background(), siteapp.LocalCLIActor(), siteapp.AcceptRestoredArchiveLoss{
			EdgeNodeID: *edgeNodeID, LedgerEpoch: *ledgerEpoch,
			ConfirmedSiteID: *confirmSiteID, Reason: *reason,
		},
	)
	if err != nil {
		return err
	}
	if !result.ArchiveLossAccepted {
		return errors.New("archive-loss decision was not applied")
	}
	return writeJSON(map[string]any{
		"status": "archive_lost", "edge_node_id": *edgeNodeID,
		"ledger_epoch": *ledgerEpoch,
	})
}

func runAccount(args []string) error {
	if len(args) == 0 {
		return errors.New("usage: iotkit-site account <bootstrap|recover> [options]")
	}
	switch args[0] {
	case "bootstrap":
		return runAccountBootstrap(args[1:])
	case "recover":
		return runAccountRecover(args[1:])
	default:
		return fmt.Errorf("unknown account command %q", args[0])
	}
}

func runAccountBootstrap(args []string) error {
	flags := flag.NewFlagSet("account bootstrap", flag.ContinueOnError)
	dbPath := flags.String("db", "site.db", "Site SQLite path")
	loginID := flags.String("login-id", "", "initial system administrator login ID")
	displayName := flags.String("display-name", "", "initial system administrator display name")
	passwordFile := flags.String("password-file", "", "owner-only file containing the password")
	if err := flags.Parse(args); err != nil {
		return err
	}
	if *loginID == "" || *displayName == "" || *passwordFile == "" {
		return errors.New("--login-id, --display-name, and --password-file are required")
	}
	password, err := readOwnerOnlySecret(*passwordFile)
	if err != nil {
		return err
	}
	service, archive, err := openAccountService(*dbPath)
	if err != nil {
		return err
	}
	defer archive.Close()
	result, err := service.DispatchAccount(
		context.Background(),
		siteapp.LocalCLIActor(),
		siteapp.CreateInitialSystemAdmin{
			LoginID:     *loginID,
			DisplayName: *displayName,
			Password:    password,
		},
	)
	if err != nil {
		return err
	}
	return writeJSON(result.Account)
}

func runAccountRecover(args []string) error {
	flags := flag.NewFlagSet("account recover", flag.ContinueOnError)
	dbPath := flags.String("db", "site.db", "Site SQLite path")
	loginID := flags.String("login-id", "", "system administrator login ID")
	passwordFile := flags.String("password-file", "", "owner-only file containing the new password")
	if err := flags.Parse(args); err != nil {
		return err
	}
	if *loginID == "" || *passwordFile == "" {
		return errors.New("--login-id and --password-file are required")
	}
	password, err := readOwnerOnlySecret(*passwordFile)
	if err != nil {
		return err
	}
	service, archive, err := openAccountService(*dbPath)
	if err != nil {
		return err
	}
	defer archive.Close()
	result, err := service.DispatchAccount(
		context.Background(),
		siteapp.LocalCLIActor(),
		siteapp.RecoverSystemAdminPassword{LoginID: *loginID, Password: password},
	)
	if err != nil {
		return err
	}
	return writeJSON(result.Account)
}

func readOwnerOnlySecret(path string) (string, error) {
	info, err := os.Lstat(path)
	if err != nil {
		return "", fmt.Errorf("stat secret file: %w", err)
	}
	if !info.Mode().IsRegular() {
		return "", errors.New("secret file must be a regular file")
	}
	if info.Mode().Perm()&0o077 != 0 {
		return "", errors.New("secret file must be owner-only")
	}
	value, err := os.ReadFile(path)
	if err != nil {
		return "", fmt.Errorf("read secret file: %w", err)
	}
	password := string(value)
	password = strings.TrimSuffix(password, "\n")
	password = strings.TrimSuffix(password, "\r")
	if password == "" {
		return "", errors.New("secret file is empty")
	}
	return password, nil
}

func requireExistingRegularFile(path string, message string) error {
	info, err := os.Stat(path)
	if err != nil || !info.Mode().IsRegular() {
		return errors.New(message)
	}
	return nil
}

func runServe(args []string) error {
	flags := flag.NewFlagSet("serve", flag.ContinueOnError)
	dbPath := flags.String("db", "site.db", "Site SQLite path")
	siteID := flags.String("site-id", "", "deployment-assigned Site source identity")
	brokerURL := flags.String("broker-url", "", "MQTT broker URL")
	clientID := flags.String("client-id", "iotkit-site", "MQTT client ID")
	username := flags.String("username", "", "MQTT username")
	passwordFile := flags.String("password-file", "", "file containing MQTT password")
	trustMode := flags.String("trust-mode", "", "MQTT TLS trust mode: system_roots or bundle_only")
	caFile := flags.String("ca-file", "", "PEM CA bundle for bundle_only trust")
	allowInsecure := flags.Bool("allow-insecure", false, "allow plain MQTT for local tests")
	httpListen := flags.String("http-listen", "127.0.0.1:8080", "private Site HTTP listen address")
	publicOrigin := flags.String("public-origin", "", "public Caddy HTTPS origin")
	developmentHTTP := flags.Bool("development-http", false, "allow loopback HTTP origin for development")
	certificateFile := flags.String("broker-certificate-file", "", "broker certificate shown in Console status")
	storageWarningPercent := flags.Int("storage-warning-percent", 90, "Site filesystem usage warning percent")
	outputBrokerURL := flags.String("output-broker-url", "", "external application MQTT broker URL")
	outputClientID := flags.String("output-client-id", "iotkit-site-output", "external MQTT client ID")
	outputUsername := flags.String("output-username", "", "external MQTT username")
	outputPasswordFile := flags.String("output-password-file", "", "file containing external MQTT password")
	outputTrustMode := flags.String("output-trust-mode", "", "external MQTT TLS trust mode")
	outputCAFile := flags.String("output-ca-file", "", "external MQTT PEM CA bundle")
	outputAllowInsecure := flags.Bool("output-allow-insecure", false, "allow plain external MQTT for local tests")
	if err := flags.Parse(args); err != nil {
		return err
	}
	if *allowInsecure && (*trustMode != "" || *caFile != "") {
		return errors.New("--allow-insecure cannot be combined with --trust-mode or --ca-file")
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
	var tlsConfig *tls.Config
	if !*allowInsecure {
		if *trustMode == "" {
			return errors.New("--trust-mode is required unless --allow-insecure is used")
		}
		tlsConfig, err = mqttsite.LoadTLSConfig(mqttsite.TrustMode(*trustMode), *caFile)
		if err != nil {
			return err
		}
	}
	var outputConfig *mqttsite.ClientConfig
	if *outputBrokerURL != "" {
		if *outputUsername == "" || *outputPasswordFile == "" {
			return errors.New("--output-username and --output-password-file are required with --output-broker-url")
		}
		outputPasswordBytes, err := os.ReadFile(*outputPasswordFile)
		if err != nil {
			return fmt.Errorf("read external MQTT password file: %w", err)
		}
		outputPassword := strings.TrimRight(string(outputPasswordBytes), "\r\n")
		if outputPassword == "" {
			return errors.New("external MQTT password file is empty")
		}
		var outputTLS *tls.Config
		if !*outputAllowInsecure {
			if *outputTrustMode == "" {
				return errors.New("--output-trust-mode is required for TLS external MQTT")
			}
			outputTLS, err = mqttsite.LoadTLSConfig(
				mqttsite.TrustMode(*outputTrustMode), *outputCAFile,
			)
			if err != nil {
				return err
			}
		}
		outputConfig = &mqttsite.ClientConfig{
			BrokerURL: *outputBrokerURL, ClientID: *outputClientID,
			Username: *outputUsername, Password: outputPassword,
			TLSConfig: outputTLS, AllowInsecure: *outputAllowInsecure,
		}
	}
	if *publicOrigin == "" {
		return errors.New("--public-origin is required")
	}
	if err := validatePrivateHTTPListen(*httpListen); err != nil {
		return err
	}

	var archive *store.Store
	if *siteID == "" {
		archive, err = store.Open(*dbPath)
	} else {
		archive, err = store.OpenWithSiteID(*dbPath, *siteID)
	}
	if err != nil {
		return fmt.Errorf("open Site store: %w", err)
	}
	defer archive.Close()
	sessions, err := sitesession.NewManager(archive, sitesession.Options{})
	if err != nil {
		return fmt.Errorf("initialize Site sessions: %w", err)
	}
	httpHandler, err := sitehttp.New(sitehttp.Config{
		Store:                 archive,
		Site:                  siteapp.NewService(archive),
		Accounts:              siteapp.NewAccountService(archive),
		Sessions:              sessions,
		PublicOrigin:          *publicOrigin,
		DevelopmentHTTP:       *developmentHTTP,
		CertificateFile:       *certificateFile,
		StorageWarningPercent: *storageWarningPercent,
	})
	if err != nil {
		return fmt.Errorf("initialize Site HTTP: %w", err)
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	runCtx, cancel := context.WithCancel(ctx)
	defer cancel()
	httpServer := &http.Server{
		Addr:              *httpListen,
		Handler:           httpHandler,
		ReadHeaderTimeout: 5 * time.Second,
		ReadTimeout:       15 * time.Second,
		WriteTimeout:      30 * time.Second,
		IdleTimeout:       60 * time.Second,
		MaxHeaderBytes:    32 * 1024,
	}
	processCount := 2
	if outputConfig != nil {
		processCount++
	}
	errorsCh := make(chan error, processCount)
	go func() {
		err := httpServer.ListenAndServe()
		if errors.Is(err, http.ErrServerClosed) {
			err = nil
		}
		errorsCh <- err
	}()
	go func() {
		config := mqttsite.ClientConfig{
			BrokerURL:     *brokerURL,
			ClientID:      *clientID,
			Username:      *username,
			Password:      password,
			TLSConfig:     tlsConfig,
			AllowInsecure: *allowInsecure,
		}
		if outputConfig != nil {
			errorsCh <- mqttsite.RunIngest(
				runCtx, config, mqttsite.Processor{Store: archive},
				archive, slog.Default(),
			)
			return
		}
		errorsCh <- mqttsite.Run(
			runCtx, config, mqttsite.Processor{Store: archive},
			archive, slog.Default(),
		)
	}()
	if outputConfig != nil {
		go func() {
			errorsCh <- mqttsite.RunOutput(runCtx, *outputConfig, archive, slog.Default())
		}()
	}
	runErr := <-errorsCh
	cancel()
	shutdownCtx, shutdownCancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer shutdownCancel()
	shutdownErr := httpServer.Shutdown(shutdownCtx)
	if runErr != nil {
		return runErr
	}
	return shutdownErr
}

func validatePrivateHTTPListen(address string) error {
	host, _, err := net.SplitHostPort(address)
	if err != nil {
		return errors.New("--http-listen must be a host:port address")
	}
	ip := net.ParseIP(host)
	if ip == nil || !ip.IsLoopback() {
		return errors.New("--http-listen must use a loopback address")
	}
	return nil
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

	service, archive, err := openSiteService(*dbPath)
	if err != nil {
		return err
	}
	defer archive.Close()
	result, err := service.Dispatch(
		context.Background(),
		siteapp.LocalCLIActor(),
		siteapp.PutSemanticMapping{Spec: spec},
	)
	if err != nil {
		return err
	}
	return writeJSON(result.SemanticMapping)
}

func runMappingDeactivate(args []string) error {
	flags := flag.NewFlagSet("mapping-deactivate", flag.ContinueOnError)
	dbPath := flags.String("db", "site.db", "Site SQLite path")
	edgeNodeID := flags.String("edge-node-id", "", "source Edge Node ID")
	seriesKey := flags.String("series-key", "", "source series key")
	if err := flags.Parse(args); err != nil {
		return err
	}
	if *edgeNodeID == "" {
		return errors.New("--edge-node-id is required")
	}
	if *seriesKey == "" {
		return errors.New("--series-key is required")
	}
	service, archive, err := openSiteService(*dbPath)
	if err != nil {
		return err
	}
	defer archive.Close()
	result, err := service.Dispatch(
		context.Background(),
		siteapp.LocalCLIActor(),
		siteapp.DeactivateSemanticMapping{EdgeNodeID: *edgeNodeID, SeriesKey: *seriesKey},
	)
	if err != nil {
		return err
	}
	return writeJSON(result.SemanticMapping)
}

func runMappingList(args []string) error {
	flags := flag.NewFlagSet("mapping-list", flag.ContinueOnError)
	dbPath := flags.String("db", "site.db", "Site SQLite path")
	if err := flags.Parse(args); err != nil {
		return err
	}
	service, archive, err := openSiteService(*dbPath)
	if err != nil {
		return err
	}
	defer archive.Close()
	mappings, err := service.ListSemanticMappings(context.Background())
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
	operation := siteapp.PutLegacyMQTTRoute{MappingID: *mappingID, Topic: *topic}
	if err := operation.Validate(); err != nil {
		return err
	}
	service, archive, err := openSiteService(*dbPath)
	if err != nil {
		return err
	}
	defer archive.Close()
	result, err := service.Dispatch(context.Background(), siteapp.LocalCLIActor(), operation)
	if err != nil {
		return err
	}
	return writeJSON(result.LegacyMQTTRoute)
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

func openSiteService(dbPath string) (*siteapp.Service, *store.Store, error) {
	archive, err := store.Open(dbPath)
	if err != nil {
		return nil, nil, err
	}
	return siteapp.NewService(archive), archive, nil
}

func openAccountService(dbPath string) (*siteapp.AccountService, *store.Store, error) {
	archive, err := store.Open(dbPath)
	if err != nil {
		return nil, nil, err
	}
	return siteapp.NewAccountService(archive), archive, nil
}
