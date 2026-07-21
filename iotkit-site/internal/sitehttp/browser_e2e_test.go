package sitehttp

import (
	"context"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteauth"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/sitesession"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/store"
)

const browserE2EPassword = "現場担当者の 十分に長いパスワード"

func TestConsoleOperatorJourneyInBrowser(t *testing.T) {
	if os.Getenv("IOTKIT_RUN_BROWSER_E2E") != "1" {
		t.Skip("set IOTKIT_RUN_BROWSER_E2E=1 to run the Chromium journey")
	}

	archive, err := store.Open(filepath.Join(t.TempDir(), "site.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = archive.Close() })
	seedBrowserAccount(t, archive, "operator", "第一工場 設定担当者", siteapp.AccountRoleAdmin)
	seedBrowserAccount(t, archive, "viewer", "第一工場 閲覧担当者", siteapp.AccountRoleViewer)
	seedBrowserAccount(t, archive, "owner", "第一工場 システム管理者", siteapp.AccountRoleSystemAdmin)
	seedSetupDevice(t, archive)
	seedAdditionalDiscoveredEdge(t, archive)

	testServer := httptest.NewUnstartedServer(nil)
	origin := "http://" + testServer.Listener.Addr().String()
	sessions, err := sitesession.NewManager(archive, sitesession.Options{
		Delay: func(context.Context, time.Duration) error { return nil },
	})
	if err != nil {
		t.Fatal(err)
	}
	handler, err := New(Config{
		Store:           archive,
		Site:            siteapp.NewService(archive),
		Accounts:        siteapp.NewAccountService(archive),
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
		"IOTKIT_SITE_E2E_URL="+origin,
		"IOTKIT_SITE_E2E_PASSWORD="+browserE2EPassword,
	)
	output, err := command.CombinedOutput()
	if err != nil {
		t.Fatalf("browser journey failed: %v\n%s", err, strings.TrimSpace(string(output)))
	}
	t.Log(strings.TrimSpace(string(output)))
}

func seedBrowserAccount(
	t *testing.T,
	archive *store.Store,
	loginID string,
	displayName string,
	role siteapp.AccountRole,
) {
	t.Helper()
	passwordPHC, err := siteauth.HashPassword(browserE2EPassword)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.CreateSiteAccount(
		context.Background(),
		siteapp.LocalCLIActor(),
		siteapp.AccountProvision{
			LoginID:     loginID,
			DisplayName: displayName,
			Role:        role,
			PasswordPHC: passwordPHC,
		},
	); err != nil {
		t.Fatal(err)
	}
}
