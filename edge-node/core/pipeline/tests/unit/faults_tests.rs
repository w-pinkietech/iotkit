use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::status::FaultKind;

fn changes(faults: &DeviceFaults) -> Arc<AtomicUsize> {
    let counter = Arc::new(AtomicUsize::new(0));
    let seen = counter.clone();
    faults.set_listener(move || {
        seen.fetch_add(1, Ordering::SeqCst);
    });
    counter
}

#[test]
fn storage_fault_starts_once_counts_discards_and_recovers_on_success() {
    let faults = DeviceFaults::default();
    let changes = changes(&faults);

    faults.storage_write_succeeded();
    assert_eq!(changes.load(Ordering::SeqCst), 0, "no fault, no change");

    faults.storage_write_failed("database or disk is full", 8_150_000);
    faults.storage_write_failed("database or disk is full", 8_150_500);
    faults.storage_write_failed("disk I/O error", 8_151_000);
    let snapshot = faults.snapshot();
    assert!(snapshot.degraded());
    assert_eq!(
        snapshot.storage,
        Some(StorageWriteFault {
            since_uptime_ms: 8_150_000,
            count: 3,
            last_error: "disk I/O error".into(),
        })
    );
    assert_eq!(
        changes.load(Ordering::SeqCst),
        1,
        "only the start is a change"
    );

    faults.storage_write_succeeded();
    assert!(!faults.snapshot().degraded());
    assert_eq!(changes.load(Ordering::SeqCst), 2);
}

#[test]
fn interface_fault_keeps_its_start_across_repeated_failures_and_recovers_when_opened() {
    let faults = DeviceFaults::default();
    let changes = changes(&faults);

    faults.interface_open_failed(
        "bravepi_main",
        InterfaceOpenReason::NotFound,
        Some("/dev/ttyAMA0".into()),
        8_200_000,
    );
    faults.interface_open_failed(
        "bravepi_main",
        InterfaceOpenReason::NotFound,
        Some("/dev/ttyAMA0".into()),
        8_260_000,
    );
    assert_eq!(changes.load(Ordering::SeqCst), 1);
    assert_eq!(
        faults.snapshot().interfaces["bravepi_main"],
        InterfaceOpenFault {
            since_uptime_ms: 8_200_000,
            reason: InterfaceOpenReason::NotFound,
            detail: Some("/dev/ttyAMA0".into()),
        }
    );

    // The same adapter failing differently is a change but not a new start.
    faults.interface_open_failed(
        "bravepi_main",
        InterfaceOpenReason::PermissionDenied,
        Some("/dev/ttyAMA0".into()),
        8_320_000,
    );
    assert_eq!(changes.load(Ordering::SeqCst), 2);
    assert_eq!(
        faults.snapshot().interfaces["bravepi_main"].since_uptime_ms,
        8_200_000
    );

    faults.interface_opened("bravepi_main");
    faults.interface_opened("bravepi_main");
    assert!(faults.snapshot().interfaces.is_empty());
    assert_eq!(changes.load(Ordering::SeqCst), 3);
}

#[test]
fn snapshot_lists_storage_first_then_adapters_by_name_and_derives_the_wall_clock_start() {
    let faults = DeviceFaults::default();
    faults.interface_open_failed("zeta", InterfaceOpenReason::Busy, None, 100);
    faults.interface_open_failed("alpha", InterfaceOpenReason::IoError, Some("x".into()), 200);
    faults.storage_write_failed("full", 300);

    let listed = faults.snapshot().faults(InputTime {
        uptime_ms: 1_000,
        unix_epoch_ms: Some(1_784_190_000_000),
    });
    let kinds: Vec<&FaultKind> = listed.iter().map(|fault| &fault.kind).collect();
    assert!(matches!(
        kinds[0],
        FaultKind::StorageWriteFailed { count: 1 }
    ));
    assert!(
        matches!(kinds[1], FaultKind::InterfaceOpenFailed { adapter, .. } if adapter == "alpha")
    );
    assert!(
        matches!(kinds[2], FaultKind::InterfaceOpenFailed { adapter, .. } if adapter == "zeta")
    );
    assert_eq!(
        listed[0].since,
        InputTime {
            uptime_ms: 300,
            unix_epoch_ms: Some(1_784_190_000_000 - 700),
        }
    );
    assert_eq!(listed[0].detail.as_deref(), Some("full"));

    let untrusted = faults.snapshot().faults(InputTime {
        uptime_ms: 1_000,
        unix_epoch_ms: None,
    });
    assert!(
        untrusted
            .iter()
            .all(|fault| fault.since.unix_epoch_ms.is_none())
    );
}

#[test]
fn detail_is_truncated_on_a_character_boundary() {
    let faults = DeviceFaults::default();
    let long = "あ".repeat(MAX_FAULT_DETAIL_BYTES);
    faults.storage_write_failed(long, 1);
    let detail = faults.snapshot().storage.unwrap().last_error;
    assert!(detail.len() <= MAX_FAULT_DETAIL_BYTES);
    assert!(detail.chars().all(|c| c == 'あ'));
}
