package edgehttp

import (
	"context"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeauth"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgesession"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/store"
)

const browserE2EPassword = "現場担当者の 十分に長いパスワード"

func TestConsoleOperatorJourneyInBrowser(t *testing.T) {
	if os.Getenv("IOTKIT_RUN_BROWSER_E2E") != "1" {
		t.Skip("set IOTKIT_RUN_BROWSER_E2E=1 to run the Chromium journey")
	}

	archive, err := openBrowserE2EStore(t)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = archive.Close() })
	seedBrowserAccount(t, archive, "operator", "第一工場 設定担当者", edgeapp.AccountRoleAdmin)
	seedBrowserAccount(t, archive, "viewer", "第一工場 閲覧担当者", edgeapp.AccountRoleViewer)
	seedBrowserAccount(t, archive, "owner", "第一工場 システム管理者", edgeapp.AccountRoleSystemAdmin)
	seedSetupDevice(t, archive)
	seedAdditionalDiscoveredEdge(t, archive)

	testServer := httptest.NewUnstartedServer(nil)
	origin := "http://" + testServer.Listener.Addr().String()
	sessions, err := edgesession.NewManager(archive, edgesession.Options{
		Delay: func(context.Context, time.Duration) error { return nil },
	})
	if err != nil {
		t.Fatal(err)
	}
	handler, err := New(Config{
		Store:           archive,
		Edge:            edgeapp.NewService(archive),
		Accounts:        edgeapp.NewAccountService(archive),
		Sessions:        sessions,
		PublicOrigin:    origin,
		DevelopmentHTTP: true,
	})
	if err != nil {
		t.Fatal(err)
	}
	testServer.Config.Handler = handler
	testServer.Start()
	t.Cleanup(testServer.Close)

	script := filepath.Join("..", "..", "frontend", "e2e", "console-journey.mjs")
	command := exec.Command("node", script)
	command.Env = append(os.Environ(),
		"IOTKIT_EDGE_E2E_URL="+origin,
		"IOTKIT_EDGE_E2E_PASSWORD="+browserE2EPassword,
	)
	output, err := command.CombinedOutput()
	if err != nil {
		t.Fatalf("browser journey failed: %v\n%s", err, strings.TrimSpace(string(output)))
	}
	t.Log(strings.TrimSpace(string(output)))
}

func openBrowserE2EStore(t *testing.T) (*store.Store, error) {
	t.Helper()
	if postgresDSN := os.Getenv("IOTKIT_TEST_CONSOLE_POSTGRES_DSN"); postgresDSN != "" {
		return store.OpenWithOptions(store.OpenOptions{
			Profile:     store.ProfilePostgres,
			PostgresDSN: postgresDSN,
		})
	}
	return store.Open(filepath.Join(t.TempDir(), "edge.db"))
}

func seedBrowserAccount(
	t *testing.T,
	archive *store.Store,
	loginID string,
	displayName string,
	role edgeapp.AccountRole,
) {
	t.Helper()
	passwordPHC, err := edgeauth.HashPassword(browserE2EPassword)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.CreateEdgeAccount(
		context.Background(),
		edgeapp.LocalCLIActor(),
		edgeapp.AccountProvision{
			LoginID:     loginID,
			DisplayName: displayName,
			Role:        role,
			PasswordPHC: passwordPHC,
		},
	); err != nil {
		t.Fatal(err)
	}
}
