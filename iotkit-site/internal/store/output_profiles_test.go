package store

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"path/filepath"
	"regexp"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/outputadapter"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/semantics"
	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
)

func TestOpenRejectsCorruptSiteIdentityInsteadOfRegenerating(t *testing.T) {
	path := filepath.Join(t.TempDir(), "site.db")
	archive, err := Open(path)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.db.Exec(`
		UPDATE site_meta SET site_id = '現場A' WHERE singleton = 1
	`); err != nil {
		t.Fatal(err)
	}
	if err := archive.Close(); err != nil {
		t.Fatal(err)
	}
	reopened, err := Open(path)
	if err == nil {
		_ = reopened.Close()
		t.Fatal("corrupt Site identity was accepted")
	}
	db, openErr := sql.Open("sqlite", path)
	if openErr != nil {
		t.Fatal(openErr)
	}
	defer db.Close()
	var siteID string
	if queryErr := db.QueryRow(`
		SELECT site_id FROM site_meta WHERE singleton = 1
	`).Scan(&siteID); queryErr != nil {
		t.Fatal(queryErr)
	}
	if siteID != "現場A" {
		t.Fatalf("corrupt identity was silently regenerated as %q", siteID)
	}
}

func TestActivateGenericExportProfileBindsAllRulesWithSiteOwnedIdentity(
	t *testing.T,
) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	rules := createOutputProfileRules(t, archive, signalRef)

	profile, err := archive.ActivateExportProfile(
		context.Background(),
		siteapp.LocalCLIActor(),
		"IoTKit共通MQTT",
		"iotkit.mqtt-json.v1",
	)
	if err != nil {
		t.Fatal(err)
	}
	if profile.State != siteapp.ExportProfileActive ||
		!profile.AutoBindFutureRules ||
		len(profile.Bindings) != len(rules) {
		t.Fatalf("profile=%#v", profile)
	}
	var siteID string
	if err := archive.db.QueryRow(`
		SELECT site_id FROM site_meta WHERE singleton = 1
	`).Scan(&siteID); err != nil {
		t.Fatal(err)
	}
	signalPattern := regexp.MustCompile(`^sig-[0-9a-f]{32}$`)
	seen := map[string]struct{}{}
	for _, binding := range profile.Bindings {
		if binding.State != siteapp.OutputBindingActive ||
			binding.SourceID != siteID ||
			!signalPattern.MatchString(binding.SignalID) {
			t.Fatalf("binding=%#v", binding)
		}
		if _, duplicate := seen[binding.SignalID]; duplicate {
			t.Fatalf("duplicate signal ID %q", binding.SignalID)
		}
		seen[binding.SignalID] = struct{}{}
	}
	routes, err := archive.ListOutputRoutes(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(routes) != len(rules) {
		t.Fatalf("routes=%d, want %d", len(routes), len(rules))
	}
}

func TestViewerCannotActivateExportProfileThroughRepository(t *testing.T) {
	archive := openTestStore(t)
	_, err := archive.ActivateExportProfile(
		context.Background(),
		siteapp.Actor{
			Class: siteapp.ActorAccount,
			Ref:   "acct_0123456789abcdef0123456789abcdef",
			Role:  siteapp.AccountRoleViewer,
		},
		"汎用MQTT JSON",
		"iotkit.mqtt-json.v1",
	)
	if !errors.Is(err, siteapp.ErrForbidden) {
		t.Fatalf("viewer activation error=%v", err)
	}
}

func TestActivateExportProfileRollsBackWhenSignalIdentityEntropyFails(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	configuration, err := archive.GetSemanticConfiguration(
		context.Background(), signalRef,
	)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.CreateSemanticRule(
		context.Background(),
		siteapp.LocalCLIActor(),
		signalRef,
		"温度",
		semantics.RuleSpec{Kind: semantics.KindNumeric},
		siteapp.RevisionPrecondition{Expected: &configuration.Revision},
	); err != nil {
		t.Fatal(err)
	}
	original := outputRandomRead
	outputRandomRead = func([]byte) (int, error) {
		return 0, errors.New("entropy unavailable")
	}
	t.Cleanup(func() { outputRandomRead = original })
	if _, err := archive.ActivateExportProfile(
		context.Background(),
		siteapp.LocalCLIActor(),
		"IoTKit共通MQTT",
		"iotkit.mqtt-json.v1",
	); err == nil {
		t.Fatal("activation succeeded without signal identity entropy")
	}
	var profiles, identities, bindings, routes int
	for table, target := range map[string]*int{
		"export_profiles":              &profiles,
		"output_signal_identities":     &identities,
		"output_profile_rule_bindings": &bindings,
		"output_routes":                &routes,
	} {
		if err := archive.db.QueryRow(
			"SELECT count(*) FROM " + table,
		).Scan(target); err != nil {
			t.Fatal(err)
		}
	}
	if profiles != 0 || identities != 0 || bindings != 0 || routes != 0 {
		t.Fatalf(
			"partial activation profiles=%d identities=%d bindings=%d routes=%d",
			profiles, identities, bindings, routes,
		)
	}
}

func TestActivateYokaKitProfileClassifiesRulesWithoutGuessingBooleanPurpose(
	t *testing.T,
) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	rules := createOutputProfileRules(t, archive, signalRef)

	profile, err := archive.ActivateExportProfile(
		context.Background(),
		siteapp.LocalCLIActor(),
		"YokaKit",
		"yokakit.mqtt.v1",
	)
	if err != nil {
		t.Fatal(err)
	}
	states := map[string]siteapp.OutputBindingState{}
	for _, binding := range profile.Bindings {
		states[binding.RuleID] = binding.State
	}
	if states[rules["numeric"].ID] != siteapp.OutputBindingIneligible ||
		states[rules["boolean"].ID] != siteapp.OutputBindingNeedsConfiguration ||
		states[rules["cumulative"].ID] != siteapp.OutputBindingPrepared ||
		states[rules["alarm"].ID] != siteapp.OutputBindingPrepared {
		t.Fatalf("states=%#v", states)
	}
}

func TestPreviewExportProfileActivationNamesEveryAffectedRule(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	createOutputProfileRules(t, archive, signalRef)
	preview, err := archive.PreviewExportProfileActivation(
		context.Background(),
		"yokakit.mqtt.v1",
	)
	if err != nil {
		t.Fatal(err)
	}
	if preview.AutomaticCount != 2 ||
		preview.NeedsConfigurationCount != 1 ||
		preview.IneligibleCount != 1 ||
		len(preview.Rules) != 4 {
		t.Fatalf("preview=%#v", preview)
	}
	for _, rule := range preview.Rules {
		if rule.RuleID == "" || rule.DisplayName == "" ||
			rule.SensorName == "" || rule.Kind == "" ||
			rule.Disposition == "" {
			t.Fatalf("unnamed preview rule=%#v", rule)
		}
	}
}

func TestCreateSemanticRuleAutoBindsActiveProfilesInSameTransaction(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	if _, err := archive.ActivateExportProfile(
		context.Background(),
		siteapp.LocalCLIActor(),
		"IoTKit共通MQTT",
		"iotkit.mqtt-json.v1",
	); err != nil {
		t.Fatal(err)
	}
	configuration, err := archive.GetSemanticConfiguration(
		context.Background(), signalRef,
	)
	if err != nil {
		t.Fatal(err)
	}
	rule, err := archive.CreateSemanticRule(
		context.Background(),
		siteapp.LocalCLIActor(),
		signalRef,
		"温度",
		semantics.RuleSpec{Kind: semantics.KindNumeric},
		siteapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	profiles, err := archive.ListExportProfiles(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(profiles) != 1 || len(profiles[0].Bindings) != 1 ||
		profiles[0].Bindings[0].RuleID != rule.ID ||
		profiles[0].Bindings[0].State != siteapp.OutputBindingActive {
		t.Fatalf("profiles=%#v", profiles)
	}
}

func TestCreateSemanticRuleAutoBindsPreparingYokaKitProfile(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	profile, err := archive.ActivateExportProfile(
		context.Background(),
		siteapp.LocalCLIActor(),
		"YokaKit",
		"yokakit.mqtt.v1",
	)
	if err != nil {
		t.Fatal(err)
	}
	if profile.State != siteapp.ExportProfilePreparing ||
		len(profile.Bindings) != 0 {
		t.Fatalf("initial profile=%#v", profile)
	}
	configuration, err := archive.GetSemanticConfiguration(
		context.Background(), signalRef,
	)
	if err != nil {
		t.Fatal(err)
	}
	rule, err := archive.CreateSemanticRule(
		context.Background(),
		siteapp.LocalCLIActor(),
		signalRef,
		"累積値",
		semantics.RuleSpec{
			Kind: semantics.KindCumulativeCounter,
			Detector: semantics.Detector{
				Mode: semantics.DetectorBooleanHighActive,
			},
			Trigger: semantics.TriggerTransition,
		},
		siteapp.RevisionPrecondition{Expected: &configuration.Revision},
	)
	if err != nil {
		t.Fatal(err)
	}
	profiles, err := archive.ListExportProfiles(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(profiles) != 1 || len(profiles[0].Bindings) != 1 ||
		profiles[0].Bindings[0].RuleID != rule.ID ||
		profiles[0].Bindings[0].State != siteapp.OutputBindingPrepared {
		t.Fatalf("profiles=%#v", profiles)
	}
}

func TestConfigureYokaKitBooleanBindingIssuesIdentityAndRoute(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	rules := createOutputProfileRules(t, archive, signalRef)
	profile, err := archive.ActivateExportProfile(
		context.Background(),
		siteapp.LocalCLIActor(),
		"YokaKit",
		"yokakit.mqtt.v1",
	)
	if err != nil {
		t.Fatal(err)
	}
	var pending siteapp.OutputProfileRuleBinding
	for _, binding := range profile.Bindings {
		if binding.RuleID == rules["boolean"].ID {
			pending = binding
		}
	}
	configured, err := archive.ConfigureYokaKitBooleanBinding(
		context.Background(),
		siteapp.LocalCLIActor(),
		pending.BindingID,
		"gantt_chart",
		pending.Revision,
	)
	if err != nil {
		t.Fatal(err)
	}
	if configured.State != siteapp.OutputBindingPrepared ||
		configured.Mode != "gantt_chart" ||
		!regexp.MustCompile(`^sig-[0-9a-f]{32}$`).MatchString(configured.SignalID) {
		t.Fatalf("configured=%#v", configured)
	}
	routes, err := archive.ListOutputRoutes(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	var found bool
	for _, route := range routes {
		if route.RuleID == rules["boolean"].ID &&
			route.AdapterID == "yokakit.mqtt.v1" && !route.Active {
			found = true
		}
	}
	if !found {
		t.Fatalf("routes=%#v", routes)
	}
	started, err := archive.StartPreparedOutputBinding(
		context.Background(),
		siteapp.LocalCLIActor(),
		configured.BindingID,
		configured.Revision,
	)
	if err != nil {
		t.Fatal(err)
	}
	if started.State != siteapp.OutputBindingActive {
		t.Fatalf("started=%#v", started)
	}
}

func TestConfigureYokaKitBooleanBindingRejectsRetiredRule(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	rules := createOutputProfileRules(t, archive, signalRef)
	profile, err := archive.ActivateExportProfile(
		context.Background(),
		siteapp.LocalCLIActor(),
		"YokaKit",
		"yokakit.mqtt.v1",
	)
	if err != nil {
		t.Fatal(err)
	}
	var pending siteapp.OutputProfileRuleBinding
	for _, binding := range profile.Bindings {
		if binding.RuleID == rules["boolean"].ID {
			pending = binding
		}
	}
	ruleRevision := rules["boolean"].Revision
	if _, err := archive.RetireSemanticRule(
		context.Background(),
		siteapp.LocalCLIActor(),
		rules["boolean"].ID,
		siteapp.RevisionPrecondition{Expected: &ruleRevision},
	); err != nil {
		t.Fatal(err)
	}
	_, err = archive.ConfigureYokaKitBooleanBinding(
		context.Background(),
		siteapp.LocalCLIActor(),
		pending.BindingID,
		"onoff",
		pending.Revision,
	)
	if !errors.Is(err, siteapp.ErrNotFound) {
		t.Fatalf("configure retired rule error=%v", err)
	}
	routes, err := archive.ListOutputRoutes(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	for _, route := range routes {
		if route.RuleID == rules["boolean"].ID {
			t.Fatalf("retired rule received route: %#v", route)
		}
	}
}

func TestRequestExportProfileStopMovesProfileAndActiveBindingsToDraining(
	t *testing.T,
) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	createOutputProfileRules(t, archive, signalRef)
	profile, err := archive.ActivateExportProfile(
		context.Background(),
		siteapp.LocalCLIActor(),
		"IoTKit共通MQTT",
		"iotkit.mqtt-json.v1",
	)
	if err != nil {
		t.Fatal(err)
	}
	stopped, err := archive.RequestExportProfileStop(
		context.Background(),
		siteapp.LocalCLIActor(),
		profile.ProfileID,
		profile.Revision,
	)
	if err != nil {
		t.Fatal(err)
	}
	if stopped.State != siteapp.ExportProfileDraining {
		t.Fatalf("profile state=%q", stopped.State)
	}
	for _, binding := range stopped.Bindings {
		if binding.State != siteapp.OutputBindingDraining {
			t.Fatalf("binding=%#v", binding)
		}
	}
}

func TestReconcileExportProfileLifecycleStopsFullyDrainedProfile(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	configuration, err := archive.GetSemanticConfiguration(
		context.Background(), signalRef,
	)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.CreateSemanticRule(
		context.Background(),
		siteapp.LocalCLIActor(),
		signalRef,
		"温度",
		semantics.RuleSpec{Kind: semantics.KindNumeric},
		siteapp.RevisionPrecondition{Expected: &configuration.Revision},
	); err != nil {
		t.Fatal(err)
	}
	profile, err := archive.ActivateExportProfile(
		context.Background(),
		siteapp.LocalCLIActor(),
		"IoTKit共通MQTT",
		"iotkit.mqtt-json.v1",
	)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.RequestExportProfileStop(
		context.Background(),
		siteapp.LocalCLIActor(),
		profile.ProfileID,
		profile.Revision,
	); err != nil {
		t.Fatal(err)
	}
	if err := archive.ReconcileExportProfileLifecycle(context.Background()); err != nil {
		t.Fatal(err)
	}
	profiles, err := archive.ListExportProfiles(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(profiles) != 1 ||
		profiles[0].State != siteapp.ExportProfileStopped ||
		profiles[0].Bindings[0].State != siteapp.OutputBindingStopped {
		t.Fatalf("profiles=%#v", profiles)
	}
}

func TestReconcileExportProfileLifecycleIgnoresUnrelatedAcceptedRecords(
	t *testing.T,
) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	configuration, err := archive.GetSemanticConfiguration(
		context.Background(), signalRef,
	)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.CreateSemanticRule(
		context.Background(),
		siteapp.LocalCLIActor(),
		signalRef,
		"温度",
		semantics.RuleSpec{Kind: semantics.KindNumeric},
		siteapp.RevisionPrecondition{Expected: &configuration.Revision},
	); err != nil {
		t.Fatal(err)
	}
	profile, err := archive.ActivateExportProfile(
		context.Background(),
		siteapp.LocalCLIActor(),
		"IoTKit共通MQTT",
		"iotkit.mqtt-json.v1",
	)
	if err != nil {
		t.Fatal(err)
	}
	unrelated := []byte(
		`{"family":"measurement","schema_version":1,"epoch":"epoch-a",` +
			`"pub_seq":2,"series_key":"another-sensor","values":[1],` +
			`"event_time":2000}`,
	)
	if _, err := archive.db.Exec(`
		INSERT INTO raw_records(
			edge_node_id, ledger_epoch, pub_seq, publication_id,
			record_json, record_sha256, received_at
		) VALUES ('edge-node-01', 'epoch-a', 2, 'unrelated', ?, X'00', 2000);
		UPDATE accepted_cursors SET accepted_through = 2
		WHERE edge_node_id = 'edge-node-01' AND ledger_epoch = 'epoch-a'
	`, unrelated); err != nil {
		t.Fatal(err)
	}
	if _, err := archive.RequestExportProfileStop(
		context.Background(),
		siteapp.LocalCLIActor(),
		profile.ProfileID,
		profile.Revision,
	); err != nil {
		t.Fatal(err)
	}
	if err := archive.ReconcileExportProfileLifecycle(context.Background()); err != nil {
		t.Fatal(err)
	}
	profiles, err := archive.ListExportProfiles(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if profiles[0].State != siteapp.ExportProfileStopped {
		t.Fatalf("profile waited for unrelated record: %#v", profiles[0])
	}
}

func TestReAddStoppedExportProfileReusesLogicalSignalIdentity(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	configuration, err := archive.GetSemanticConfiguration(
		context.Background(), signalRef,
	)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.CreateSemanticRule(
		context.Background(),
		siteapp.LocalCLIActor(),
		signalRef,
		"温度",
		semantics.RuleSpec{Kind: semantics.KindNumeric},
		siteapp.RevisionPrecondition{Expected: &configuration.Revision},
	); err != nil {
		t.Fatal(err)
	}
	first, err := archive.ActivateExportProfile(
		context.Background(),
		siteapp.LocalCLIActor(),
		"汎用MQTT JSON",
		"iotkit.mqtt-json.v1",
	)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := archive.RequestExportProfileStop(
		context.Background(),
		siteapp.LocalCLIActor(),
		first.ProfileID,
		first.Revision,
	); err != nil {
		t.Fatal(err)
	}
	if err := archive.ReconcileExportProfileLifecycle(context.Background()); err != nil {
		t.Fatal(err)
	}
	second, err := archive.ActivateExportProfile(
		context.Background(),
		siteapp.LocalCLIActor(),
		"汎用MQTT JSON（再追加）",
		"iotkit.mqtt-json.v1",
	)
	if err != nil {
		t.Fatal(err)
	}
	if second.ProfileID == first.ProfileID ||
		second.Bindings[0].BindingID == first.Bindings[0].BindingID {
		t.Fatalf("lifecycle identity was reused first=%#v second=%#v",
			first, second)
	}
	if second.Bindings[0].SignalID != first.Bindings[0].SignalID {
		t.Fatalf("signal_id changed first=%q second=%q",
			first.Bindings[0].SignalID, second.Bindings[0].SignalID)
	}
}

func TestReAddYokaKitBindingReusesOnlyTheSameModeSignalIdentity(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	rules := createOutputProfileRules(t, archive, signalRef)
	booleanRule := rules["boolean"]

	activateAndConfigure := func(
		displayName string,
		mode outputadapter.YokaKitKind,
	) (siteapp.ExportProfile, siteapp.OutputProfileRuleBinding) {
		t.Helper()
		profile, err := archive.ActivateExportProfile(
			context.Background(),
			siteapp.LocalCLIActor(),
			displayName,
			"yokakit.mqtt.v1",
		)
		if err != nil {
			t.Fatal(err)
		}
		var pending siteapp.OutputProfileRuleBinding
		for _, binding := range profile.Bindings {
			if binding.RuleID == booleanRule.ID {
				pending = binding
				break
			}
		}
		if pending.BindingID == "" {
			t.Fatalf("boolean binding missing: %#v", profile)
		}
		configured, err := archive.ConfigureYokaKitBooleanBinding(
			context.Background(),
			siteapp.LocalCLIActor(),
			pending.BindingID,
			string(mode),
			pending.Revision,
		)
		if err != nil {
			t.Fatal(err)
		}
		return profile, configured
	}
	stop := func(profile siteapp.ExportProfile) {
		t.Helper()
		if _, err := archive.RequestExportProfileStop(
			context.Background(),
			siteapp.LocalCLIActor(),
			profile.ProfileID,
			profile.Revision,
		); err != nil {
			t.Fatal(err)
		}
		if err := archive.ReconcileExportProfileLifecycle(
			context.Background(),
		); err != nil {
			t.Fatal(err)
		}
	}

	firstProfile, first := activateAndConfigure(
		"YokaKit ON/OFF",
		outputadapter.YokaKitOnOff,
	)
	stop(firstProfile)
	secondProfile, second := activateAndConfigure(
		"YokaKit ON/OFF 再追加",
		outputadapter.YokaKitOnOff,
	)
	if first.BindingID == second.BindingID {
		t.Fatal("same-mode re-add reused binding_id")
	}
	if first.SignalID != second.SignalID {
		t.Fatalf("same-mode signal_id changed first=%q second=%q",
			first.SignalID, second.SignalID)
	}

	stop(secondProfile)
	_, third := activateAndConfigure(
		"YokaKit 稼働区間",
		outputadapter.YokaKitGanttChart,
	)
	if third.SignalID == second.SignalID {
		t.Fatalf("different modes shared signal_id=%q", third.SignalID)
	}
}

func TestOutputBindingPreviewReturnsSchemaCompleteAdapterPublication(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	rules := createOutputProfileRules(t, archive, signalRef)
	profile, err := archive.ActivateExportProfile(
		context.Background(),
		siteapp.LocalCLIActor(),
		"YokaKit",
		"yokakit.mqtt.v1",
	)
	if err != nil {
		t.Fatal(err)
	}
	var binding siteapp.OutputProfileRuleBinding
	for _, candidate := range profile.Bindings {
		if candidate.RuleID == rules["cumulative"].ID {
			binding = candidate
		}
	}
	preview, err := archive.GetOutputBindingPublication(
		context.Background(), binding.BindingID,
	)
	if err != nil {
		t.Fatal(err)
	}
	if preview.Provenance != "sample" ||
		preview.Topic != "yokakit/v1/sources/"+binding.SourceID+
			"/signals/"+binding.SignalID+"/observations" ||
		preview.QoS != 1 ||
		preview.Retain {
		t.Fatalf("preview=%#v", preview)
	}
	var payload map[string]any
	if err := json.Unmarshal(preview.Payload, &payload); err != nil {
		t.Fatal(err)
	}
	for _, field := range []string{
		"schema_version", "observation_id", "series_id", "sequence",
		"observed_at", "kind", "value",
	} {
		if _, exists := payload[field]; !exists {
			t.Fatalf("payload missing %s: %s", field, preview.Payload)
		}
	}
}

func TestOutputBindingPreviewMatchesDurableOutboxPublication(t *testing.T) {
	archive := openTestStore(t)
	signalRef := semanticV3Signal(t, archive)
	rules := createOutputProfileRules(t, archive, signalRef)
	profile, err := archive.ActivateExportProfile(
		context.Background(),
		siteapp.LocalCLIActor(),
		"YokaKit",
		"yokakit.mqtt.v1",
	)
	if err != nil {
		t.Fatal(err)
	}
	var binding siteapp.OutputProfileRuleBinding
	for _, candidate := range profile.Bindings {
		if candidate.RuleID == rules["cumulative"].ID {
			binding = candidate
		}
	}
	binding, err = archive.StartPreparedOutputBinding(
		context.Background(),
		siteapp.LocalCLIActor(),
		binding.BindingID,
		binding.Revision,
	)
	if err != nil {
		t.Fatal(err)
	}
	acceptSemanticBatch(
		t, archive, "edge-node-01", "epoch-a", 2,
		[]float64{0}, []float64{1},
	)
	if _, err := archive.ProjectSemanticRules(context.Background(), 100); err != nil {
		t.Fatal(err)
	}
	if _, err := archive.EnqueueMultipleRuleOutputExports(
		context.Background(), 100,
	); err != nil {
		t.Fatal(err)
	}
	pending, err := archive.ListPendingMQTTExports(context.Background(), 100)
	if err != nil {
		t.Fatal(err)
	}
	preview, err := archive.GetOutputBindingPublication(
		context.Background(), binding.BindingID,
	)
	if err != nil {
		t.Fatal(err)
	}
	var matched bool
	for _, publication := range pending {
		if publication.RouteID == "" ||
			publication.Topic != preview.Topic ||
			string(publication.PayloadJSON) != string(preview.Payload) {
			continue
		}
		matched = true
	}
	if preview.Provenance != "actual" || !matched {
		t.Fatalf("preview=%#v pending=%#v", preview, pending)
	}
}

func createOutputProfileRules(
	t *testing.T,
	archive *Store,
	signalRef string,
) map[string]semantics.Rule {
	t.Helper()
	ctx := context.Background()
	specs := []struct {
		key, name string
		spec      semantics.RuleSpec
	}{
		{"numeric", "温度", semantics.RuleSpec{Kind: semantics.KindNumeric}},
		{
			"boolean",
			"運転状態",
			semantics.RuleSpec{
				Kind: semantics.KindBoolean,
				Detector: semantics.Detector{
					Mode: semantics.DetectorBooleanHighActive,
				},
			},
		},
		{
			"cumulative",
			"累積値",
			semantics.RuleSpec{
				Kind: semantics.KindCumulativeCounter,
				Detector: semantics.Detector{
					Mode: semantics.DetectorBooleanHighActive,
				},
				Trigger: semantics.TriggerTransition,
			},
		},
		{
			"alarm",
			"高温異常",
			semantics.RuleSpec{
				Kind: semantics.KindAlarm,
				Detector: semantics.Detector{
					Mode:          semantics.DetectorHighActive,
					RiseThreshold: 80,
					FallThreshold: 75,
				},
			},
		},
	}
	result := make(map[string]semantics.Rule, len(specs))
	for _, item := range specs {
		configuration, err := archive.GetSemanticConfiguration(ctx, signalRef)
		if err != nil {
			t.Fatal(err)
		}
		rule, err := archive.CreateSemanticRule(
			ctx,
			siteapp.LocalCLIActor(),
			signalRef,
			item.name,
			item.spec,
			siteapp.RevisionPrecondition{Expected: &configuration.Revision},
		)
		if err != nil {
			t.Fatalf("create %s rule: %v", item.key, err)
		}
		result[item.key] = rule
	}
	return result
}
