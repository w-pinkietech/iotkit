use super::*;

#[test]
fn fan_in_continues_while_restart_notification_is_pending() {
    assert!(
        !should_stop_after_all_adapter_streams_closed(1, 0, false, false),
        "pending restart timers must keep the fan-in loop alive"
    );
    assert!(
        should_stop_after_all_adapter_streams_closed(0, 0, true, false),
        "normal adapter closure should stop even while the API task is running"
    );
    assert!(
        !should_stop_after_all_adapter_streams_closed(0, 0, true, true),
        "service-only mode must keep the fan-in loop alive until its background service exits"
    );
    assert!(
        should_stop_after_all_adapter_streams_closed(0, 0, false, true),
        "the fan-in loop may stop only when no restart is pending"
    );
    assert!(
        !should_stop_after_all_adapter_streams_closed(0, 1, false, false),
        "an exhausted adapter remains process-lifetime degraded"
    );
}

#[test]
fn run_exit_status_reflects_collector_api_mqtt_and_cleanup_failures() {
    assert!(
        !should_exit_nonzero(true, false, false, true),
        "requested shutdown and normal adapter closure should exit successfully"
    );
    assert!(
        should_exit_nonzero(false, false, false, true),
        "collector death remains fail-fast"
    );
    assert!(
        should_exit_nonzero(true, true, false, true),
        "unexpected API task exit in API-only mode should be fail-fast"
    );
    assert!(
        should_exit_nonzero(true, false, true, true),
        "unexpected MQTT publisher exit should be fail-fast"
    );
    assert!(
        should_exit_nonzero(true, false, false, false),
        "a timed-out cleanup must not report a clean shutdown"
    );
}

#[tokio::test]
async fn shutdown_deadline_bounds_a_stuck_cleanup() {
    assert!(
        !shutdown_with_timeout(Duration::from_millis(1), std::future::pending()).await,
        "a stuck cleanup task must not hold the process past its shutdown deadline"
    );
}

#[tokio::test]
async fn generic_running_adapter_drives_health_and_closed_lifecycle() {
    let instance_id =
        iotkit_input_adapter_host_api::AdapterInstanceId::new("reference_one").unwrap();
    let (runtime, running) = iotkit_input_adapter_host_api::runtime_channels(instance_id, 1);
    let (healthy_tx, mut healthy_rx) = tokio::sync::mpsc::channel(1);
    let mut host = AdapterHost::new();
    let device_faults = iotkit_core_pipeline::DeviceFaults::default();
    let adapter_id =
        register_running_adapter(&mut host, running, healthy_tx, 7, device_faults.clone())
            .expect("register");

    runtime.activity.physical_decode();
    runtime
        .activity
        .interface_open_failed(std::io::ErrorKind::PermissionDenied, "/dev/ttyAMA0");
    let healthy = tokio::time::timeout(Duration::from_secs(2), healthy_rx.recv())
        .await
        .expect("activity monitor timeout")
        .expect("activity monitor closed");
    assert_eq!(healthy.adapter_id, adapter_id);
    assert_eq!(healthy.generation, 7);
    let fault = wait_for_interface_fault(&device_faults, "reference_one", true).await;
    assert_eq!(
        fault.reason,
        iotkit_core_pipeline::InterfaceOpenReason::PermissionDenied
    );
    assert_eq!(fault.detail.as_deref(), Some("/dev/ttyAMA0"));

    runtime.activity.interface_opened();
    wait_for_interface_fault(&device_faults, "reference_one", false).await;

    runtime
        .completion
        .complete(AdapterCompletion::UnexpectedExit(
            iotkit_input_adapter_host_api::UnexpectedExitReason::WorkerReturned,
        ));
    let closed = tokio::time::timeout(Duration::from_secs(2), host.next_event())
        .await
        .expect("host close timeout");
    assert!(matches!(closed, Some(AdapterHostEvent::AdapterClosed(id)) if id == adapter_id));
}

/// The activity monitor samples once a second; wait until the fault for
/// `adapter` is present (or absent) and return it.
async fn wait_for_interface_fault(
    faults: &iotkit_core_pipeline::DeviceFaults,
    adapter: &str,
    present: bool,
) -> iotkit_core_pipeline::InterfaceOpenFault {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = faults.snapshot();
            match snapshot.interfaces.get(adapter) {
                Some(fault) if present => return fault.clone(),
                None if !present => {
                    return iotkit_core_pipeline::InterfaceOpenFault {
                        since_uptime_ms: 0,
                        reason: iotkit_core_pipeline::InterfaceOpenReason::IoError,
                        detail: None,
                    };
                }
                _ => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
    })
    .await
    .expect("interface fault did not reach the expected state")
}

#[test]
fn stale_activity_from_a_previous_runtime_generation_is_ignored() {
    let id = AdapterId::new("line_a");
    let active = HashMap::from([(id.clone(), 2)]);
    assert!(!activity_notice_is_current(
        &active,
        &ActivityNotice {
            adapter_id: id.clone(),
            generation: 1,
        }
    ));
    assert!(activity_notice_is_current(
        &active,
        &ActivityNotice {
            adapter_id: id,
            generation: 2,
        }
    ));
}
