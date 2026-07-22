package edgehttp

import (
	"bytes"
	"context"
	"encoding/json"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/contract"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeapp"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgeauth"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/edgesession"
	"github.com/w-pinkietech/iotkit-next/iotkit-edge/internal/store"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"testing"
	"time"
)

func newTestServer(t *testing.T, mustChangePassword bool) http.Handler {
	return newTestServerWithRole(t, mustChangePassword, edgeapp.AccountRoleViewer)
}

func newTestServerWithRole(
	t *testing.T,
	mustChangePassword bool,
	role edgeapp.AccountRole,
) http.Handler {
	server, _ := newTestServerFixture(t, mustChangePassword, role)
	return server
}

func newTestServerFixture(
	t *testing.T,
	mustChangePassword bool,
	role edgeapp.AccountRole,
) (http.Handler, *store.Store) {
	t.Helper()
	archive, err := store.Open(filepath.Join(t.TempDir(), "edge.db"))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = archive.Close() })
	passwordPHC, err := edgeauth.HashPassword("現場担当者の 十分に長いパスワード")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.CreateEdgeAccount(
		context.Background(),
		edgeapp.LocalCLIActor(),
		edgeapp.AccountProvision{
			LoginID:            "operator",
			DisplayName:        "第一工場 担当者",
			Role:               role,
			PasswordPHC:        passwordPHC,
			MustChangePassword: mustChangePassword,
		},
	); err != nil {
		t.Fatal(err)
	}
	sessions, err := edgesession.NewManager(archive, edgesession.Options{
		Delay: func(context.Context, time.Duration) error { return nil },
	})
	if err != nil {
		t.Fatal(err)
	}
	handler, err := New(Config{
		Store:        archive,
		Edge:         edgeapp.NewService(archive),
		Accounts:     edgeapp.NewAccountService(archive),
		Sessions:     sessions,
		PublicOrigin: testOrigin,
	})
	if err != nil {
		t.Fatal(err)
	}
	return handler, archive
}

func seedDiscoveredEdge(t *testing.T, archive *store.Store) edgeapp.EdgeNode {
	t.Helper()
	snapshot := contract.DescriptorSnapshot{
		SchemaVersion:      2,
		EdgeNodeID:         "factory-edge-01",
		LedgerEpoch:        "epoch-01",
		DescriptorRevision: 1,
		Complete:           true,
		Devices: []contract.DescriptorDevice{{
			SystemID: "018f0000-0000-7000-8000-000000000001",
			State:    "active",
		}},
		Signals: []contract.DescriptorSignal{{
			SeriesKey:      "018f0000-0000-7000-8000-000000000001:temperature_c:na:primary",
			SystemID:       "018f0000-0000-7000-8000-000000000001",
			MeasurementKey: "temperature_c",
			Variant:        "primary",
			ValueType:      "float",
		}},
	}
	if _, err := archive.ApplyDescriptorSnapshot(
		context.Background(), snapshot,
	); err != nil {
		t.Fatal(err)
	}
	edgeNodes, err := archive.ListEdgeNodes(context.Background())
	if err != nil || len(edgeNodes) != 1 {
		t.Fatalf("edgeNodes = %#v, err = %v", edgeNodes, err)
	}
	return edgeNodes[0]
}

func seedAdditionalDiscoveredEdge(t *testing.T, archive *store.Store) edgeapp.EdgeNode {
	t.Helper()
	const systemID = "018f0000-0000-7000-8000-000000000002"
	identifier := "BP-87654321"
	unit := "1"
	if _, err := archive.ApplyDescriptorSnapshot(
		context.Background(),
		contract.DescriptorSnapshot{
			SchemaVersion:      2,
			EdgeNodeID:         "assembly-edge-02",
			LedgerEpoch:        "epoch-02",
			DescriptorRevision: 1,
			Complete:           true,
			Devices: []contract.DescriptorDevice{{
				SystemID:   systemID,
				Identifier: &identifier,
				State:      "active",
			}},
			Signals: []contract.DescriptorSignal{{
				SeriesKey:      systemID + ":contact_state:na:primary",
				SystemID:       systemID,
				MeasurementKey: "contact_state",
				Variant:        "primary",
				Unit:           &unit,
				ValueType:      "bool",
			}},
		},
	); err != nil {
		t.Fatal(err)
	}
	edgeNodes, err := archive.ListEdgeNodes(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	for _, edgeNode := range edgeNodes {
		if edgeNode.EdgeNodeID == "assembly-edge-02" {
			return edgeNode
		}
	}
	t.Fatal("additional discovered EdgeNode was not listed")
	return edgeapp.EdgeNode{}
}

func seedSetupDevice(t *testing.T, archive *store.Store) {
	t.Helper()
	const (
		systemID  = "018f0000-0000-7000-8000-000000000001"
		seriesKey = systemID + ":temperature_c:na:primary"
	)
	identifier := "BP-01234567"
	modelID := "mcp9600"
	unit := "Cel"
	snapshot := contract.DescriptorSnapshot{
		SchemaVersion:      2,
		EdgeNodeID:         "factory-edge-01",
		LedgerEpoch:        "epoch-01",
		DescriptorRevision: 1,
		Complete:           true,
		Devices: []contract.DescriptorDevice{{
			SystemID:   systemID,
			Identifier: &identifier,
			ModelID:    &modelID,
			State:      "active",
		}},
		Signals: []contract.DescriptorSignal{{
			SeriesKey:      seriesKey,
			SystemID:       systemID,
			MeasurementKey: "temperature_c",
			Variant:        "primary",
			Unit:           &unit,
			ValueType:      "float",
		}},
	}
	if _, err := archive.ApplyDescriptorSnapshot(context.Background(), snapshot); err != nil {
		t.Fatal(err)
	}
	edgeNodes, err := archive.ListEdgeNodes(context.Background())
	if err != nil || len(edgeNodes) != 1 {
		t.Fatalf("edgeNodes = %#v, err = %v", edgeNodes, err)
	}
	expected := edgeNodes[0].Revision
	if _, err := archive.RequestEdgeNodeActivation(
		context.Background(),
		edgeapp.LocalCLIActor(),
		edgeNodes[0].EdgeNodeRef,
		edgeapp.RevisionPrecondition{Expected: &expected},
	); err != nil {
		t.Fatal(err)
	}
	commands, err := archive.ListPendingActivationCommands(context.Background(), 1)
	if err != nil || len(commands) != 1 {
		t.Fatalf("activation commands = %#v, err = %v", commands, err)
	}
	activationRequest, err := contract.DecodeActivationRequest(commands[0].PayloadJSON)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.ApplyActivationResult(
		context.Background(),
		contract.ActivationResult{
			SchemaVersion:            contract.SchemaVersion,
			ActivationID:             activationRequest.ActivationID,
			EdgeID:                   activationRequest.EdgeID,
			EdgeNodeID:               activationRequest.EdgeNodeID,
			LedgerEpoch:              activationRequest.ExpectedLedgerEpoch,
			Status:                   "applied",
			DiscardThroughReadingSeq: 0,
			FirstPublicationSeq:      1,
			AppliedAt:                activationRequest.IssuedAt + 1,
		},
	); err != nil {
		t.Fatal(err)
	}
	record := json.RawMessage(
		`{"family":"measurement","schema_version":1,"epoch":"epoch-01","pub_seq":1,` +
			`"series_key":"` + seriesKey + `","values":[24.8],"event_time":1000,` +
			`"event_time_source":"received_at","time_source":"edge_node",` +
			`"time_quality":"unsynced","received_at":1000,"device_time":null}`,
	)
	batch := contract.RecordBatch{
		SchemaVersion: 1,
		EdgeNodeID:    "factory-edge-01",
		LedgerEpoch:   "epoch-01",
		PublicationID: contract.PublicationID("factory-edge-01", "epoch-01", 1, 1),
		CursorStart:   1,
		CursorEnd:     1,
		Records:       []json.RawMessage{record},
	}
	if _, err := archive.AcceptBatch(context.Background(), batch); err != nil {
		t.Fatal(err)
	}
	if processed, err := archive.ReconcileInventorySources(context.Background(), 100); err != nil ||
		processed != 1 {
		t.Fatalf("reconcile processed=%d err=%v", processed, err)
	}
}

func loginTestAccount(t *testing.T, server http.Handler) (*http.Cookie, string) {
	t.Helper()
	body := bytes.NewBufferString(
		`{"login_id":"operator","password":"現場担当者の 十分に長いパスワード"}`,
	)
	request := httptest.NewRequest(http.MethodPost, "/api/v1/session", body)
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("Origin", testOrigin)
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	if response.Code != http.StatusCreated {
		t.Fatalf("login status = %d, body=%s", response.Code, response.Body.String())
	}
	var payload struct {
		CSRFToken string `json:"csrf_token"`
	}
	if err := json.Unmarshal(response.Body.Bytes(), &payload); err != nil {
		t.Fatal(err)
	}
	for _, cookie := range response.Result().Cookies() {
		if cookie.Name == sessionCookieName {
			return cookie, payload.CSRFToken
		}
	}
	t.Fatal("login response did not set the session cookie")
	return nil, ""
}
