package store

import (
	"context"
	"errors"
	"testing"

	"github.com/w-pinkietech/iotkit-next/iotkit-site/internal/siteapp"
)

func TestUpdateDeviceProfileCommitsRevisionAndAuditAtomically(t *testing.T) {
	store := openTestStore(t)
	if _, err := store.ApplyDescriptorSnapshot(context.Background(), descriptorFixture(t)); err != nil {
		t.Fatal(err)
	}
	expected := int64(0)
	profile, err := store.UpdateDeviceProfile(
		context.Background(),
		siteapp.LocalCLIActor(),
		testSourceRef(t, store.db, "site_devices", "device_ref"),
		siteapp.DeviceProfileInput{
			DisplayName: "乾燥炉入口",
			Location:    "第2工場・乾燥炉入口",
		},
		siteapp.RevisionPrecondition{Expected: &expected},
	)
	if err != nil {
		t.Fatal(err)
	}
	if profile.Revision != 1 {
		t.Fatalf("revision = %d, want 1", profile.Revision)
	}
	events, err := store.ListAuditEvents(context.Background(), 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(events) != 1 || events[0].Operation != "device_profile.update" ||
		events[0].ResourceRef != profile.DeviceRef {
		t.Fatalf("audit events = %#v", events)
	}
}

func TestUpdateSignalProfileCommitsRevisionAndAudit(t *testing.T) {
	store := openTestStore(t)
	if _, err := store.ApplyDescriptorSnapshot(context.Background(), descriptorFixture(t)); err != nil {
		t.Fatal(err)
	}
	expected := int64(0)
	profile, err := store.UpdateSignalProfile(
		context.Background(),
		siteapp.LocalCLIActor(),
		testSourceRef(t, store.db, "site_signals", "signal_ref"),
		testSignalProfileInput(),
		siteapp.RevisionPrecondition{Expected: &expected},
	)
	if err != nil {
		t.Fatal(err)
	}
	if profile.Revision != 1 || profile.DisplayName != "乾燥炉入口温度" ||
		profile.DisplaySensorType != "temperature" ||
		profile.DisplayValueKind != "numeric" ||
		profile.DisplayUnitMode != "unit" || profile.DisplayUnit != "°C" ||
		profile.DecimalPlaces != 1 || !profile.Complete() {
		t.Fatalf("profile = %#v", profile)
	}
	var (
		sensorType    string
		valueKind     string
		unitMode      string
		unit          string
		decimalPlaces int
	)
	if err := store.db.QueryRow(`
		SELECT display_sensor_type, display_value_kind, display_unit_mode,
			display_unit, decimal_places
		FROM signal_profiles
	`).Scan(&sensorType, &valueKind, &unitMode, &unit, &decimalPlaces); err != nil {
		t.Fatal(err)
	}
	if sensorType != "temperature" || valueKind != "numeric" ||
		unitMode != "unit" || unit != "°C" || decimalPlaces != 1 {
		t.Fatalf("stored profile = %q %q %q %q %d",
			sensorType, valueKind, unitMode, unit, decimalPlaces)
	}
	events, err := store.ListAuditEvents(context.Background(), 10)
	if err != nil {
		t.Fatal(err)
	}
	if len(events) != 1 || events[0].Operation != "signal_profile.update" ||
		events[0].ResourceRef != profile.SignalRef {
		t.Fatalf("audit events = %#v", events)
	}
}

func TestUpdateSignalProfileRollsBackWhenAuditInsertFails(t *testing.T) {
	store := openTestStore(t)
	if _, err := store.ApplyDescriptorSnapshot(context.Background(), descriptorFixture(t)); err != nil {
		t.Fatal(err)
	}
	if _, err := store.db.Exec(`
		CREATE TRIGGER fail_profile_audit
		BEFORE INSERT ON audit_events
		BEGIN SELECT RAISE(ABORT, 'fail'); END;
	`); err != nil {
		t.Fatal(err)
	}
	_, err := store.UpdateSignalProfile(
		context.Background(),
		siteapp.LocalCLIActor(),
		testSourceRef(t, store.db, "site_signals", "signal_ref"),
		testSignalProfileInput(),
		siteapp.RevisionPrecondition{},
	)
	if err == nil {
		t.Fatal("profile update succeeded despite audit failure")
	}
	if got := testTableCount(t, store.db, "signal_profiles"); got != 0 {
		t.Fatalf("signal profiles = %d, want 0", got)
	}
}

func testSignalProfileInput() siteapp.SignalProfileInput {
	return siteapp.SignalProfileInput{
		DisplayName:       "乾燥炉入口温度",
		DisplaySensorType: "temperature",
		DisplayValueKind:  "numeric",
		DisplayUnitMode:   "unit",
		DisplayUnit:       "°C",
		DecimalPlaces:     1,
	}
}

func TestUpdateDeviceProfileRejectsStaleRevision(t *testing.T) {
	store := openTestStore(t)
	if _, err := store.ApplyDescriptorSnapshot(context.Background(), descriptorFixture(t)); err != nil {
		t.Fatal(err)
	}
	deviceRef := testSourceRef(t, store.db, "site_devices", "device_ref")
	expected := int64(0)
	if _, err := store.UpdateDeviceProfile(
		context.Background(), siteapp.LocalCLIActor(), deviceRef,
		siteapp.DeviceProfileInput{DisplayName: "乾燥炉入口", Location: "第2工場"},
		siteapp.RevisionPrecondition{Expected: &expected},
	); err != nil {
		t.Fatal(err)
	}
	_, err := store.UpdateDeviceProfile(
		context.Background(), siteapp.LocalCLIActor(), deviceRef,
		siteapp.DeviceProfileInput{DisplayName: "別名", Location: "第2工場"},
		siteapp.RevisionPrecondition{Expected: &expected},
	)
	if !errors.Is(err, siteapp.ErrRevisionMismatch) {
		t.Fatalf("stale revision error = %v", err)
	}
}
