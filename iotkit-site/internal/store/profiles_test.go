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
		siteapp.SignalProfileInput{DisplayName: "乾燥炉入口温度"},
		siteapp.RevisionPrecondition{Expected: &expected},
	)
	if err != nil {
		t.Fatal(err)
	}
	if profile.Revision != 1 || profile.DisplayName != "乾燥炉入口温度" {
		t.Fatalf("profile = %#v", profile)
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
		siteapp.SignalProfileInput{DisplayName: "乾燥炉入口温度"},
		siteapp.RevisionPrecondition{},
	)
	if err == nil {
		t.Fatal("profile update succeeded despite audit failure")
	}
	if got := testTableCount(t, store.db, "signal_profiles"); got != 0 {
		t.Fatalf("signal profiles = %d, want 0", got)
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
