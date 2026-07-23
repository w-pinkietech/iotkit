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
fn run_exit_status_reflects_collector_and_api_failures() {
    assert!(
        !should_exit_nonzero(true, false, false),
        "ctrl_c and normal adapter closure should exit successfully"
    );
    assert!(
        should_exit_nonzero(false, false, false),
        "collector death remains fail-fast"
    );
    assert!(
        should_exit_nonzero(true, true, false),
        "unexpected API task exit in API-only mode should be fail-fast"
    );
    assert!(
        should_exit_nonzero(true, false, true),
        "unexpected MQTT publisher exit should be fail-fast"
    );
}

#[tokio::test]
async fn generic_running_adapter_drives_health_and_closed_lifecycle() {
    let instance_id =
        iotkit_input_adapter_host_api::AdapterInstanceId::new("reference_one").unwrap();
    let (runtime, running) = iotkit_input_adapter_host_api::runtime_channels(instance_id, 1);
    let (healthy_tx, mut healthy_rx) = tokio::sync::mpsc::channel(1);
    let mut host = AdapterHost::new();
    let adapter_id = register_running_adapter(&mut host, running, healthy_tx, 7).expect("register");

    runtime.activity.physical_decode();
    let healthy = tokio::time::timeout(Duration::from_secs(2), healthy_rx.recv())
        .await
        .expect("activity monitor timeout")
        .expect("activity monitor closed");
    assert_eq!(healthy.adapter_id, adapter_id);
    assert_eq!(healthy.generation, 7);

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
