use super::*;
use iotkit_ingest_client::{QueueSubmitError, channel_for_test};
use iotkit_ingest_contract::{ReadingItem, TimeSource};

fn item() -> ReadingItem {
    ReadingItem {
        subject_hint: Some("sensor:1".into()),
        measurement_key: "temperature_c".into(),
        channel_index: None,
        series_variant: None,
        values: vec![21.5],
        device_time_ms: None,
        time_source: TimeSource::EdgeNode,
        age_ms: None,
        rssi: None,
        battery_pct: None,
    }
}

#[test]
fn identifiers_use_the_closed_contract_syntax() {
    for valid in ["a", "bravepi-mainboard", "rpi-local"] {
        assert!(AdapterTypeId::new(valid).is_ok(), "{valid}");
    }
    for invalid in ["", "A", "a_b", "-a", "a-", "a--b", "日本語"] {
        assert!(AdapterTypeId::new(invalid).is_err(), "{invalid}");
    }
    for valid in ["a", "bravepi_main", "line-1"] {
        assert!(AdapterInstanceId::new(valid).is_ok(), "{valid}");
    }
    for invalid in ["", "A", "_a", "a_", "a__b", "日本語"] {
        assert!(AdapterInstanceId::new(invalid).is_err(), "{invalid}");
    }
    assert!(ConfiguredSource::new("input:vendor/device.1").is_ok());
    assert!(ConfiguredSource::new("bad source").is_err());
}

#[tokio::test]
async fn source_bound_ingest_owns_source_and_preserves_retry_envelope() {
    let (client, mut receiver) = channel_for_test(1);
    let source = ConfiguredSource::new("input:test:one").unwrap();
    let bound = SourceBoundIngest::new(source.clone(), client);
    let first = bound.try_submit(vec![item()]).unwrap();
    let envelope = receiver.recv().await.unwrap();
    assert_eq!(envelope.source, source.as_str());
    assert_eq!(envelope.envelope_id, first.envelope_id);

    drop(receiver);
    let QueueSubmitError::Closed(retry) = bound.try_submit(vec![item()]).unwrap_err() else {
        panic!("closed queue must return retry ownership");
    };
    let retry_id = retry.envelope_id().to_owned();
    let RetryQueueError::Closed(retry) = bound.try_retry(retry).unwrap_err() else {
        panic!("retry must preserve a closed result");
    };
    assert_eq!(retry.envelope_id(), retry_id);
}

#[tokio::test]
async fn runtime_channels_coalesce_activity_bound_diagnostics_and_complete_once() {
    let id = AdapterInstanceId::new("test_one").unwrap();
    let (runtime, running) = runtime_channels(id.clone(), 1);
    runtime.activity.physical_decode();
    runtime.activity.physical_decode();
    assert!(running.activity.snapshot().last_physical_decode.is_some());

    runtime
        .diagnostics
        .try_emit(AdapterDiagnostic::new(
            DiagnosticKind::Decode,
            "invalid frame",
        ))
        .unwrap();
    assert!(
        runtime
            .diagnostics
            .try_emit(AdapterDiagnostic::new(DiagnosticKind::Transport, "closed",))
            .is_err()
    );
    assert_eq!(running.activity.snapshot().dropped_diagnostics, 1);

    running.shutdown.request();
    assert!(runtime.stop.is_requested());
    runtime
        .completion
        .complete(AdapterCompletion::RequestedStop);
    assert_eq!(
        running.completion.wait().await,
        AdapterCompletion::RequestedStop
    );
    assert_eq!(running.instance_id, id);
}

#[test]
fn start_context_rejects_mismatched_configured_and_ingest_sources() {
    let (client, _receiver) = channel_for_test(1);
    let configured_source = ConfiguredSource::new("input:test:configured").unwrap();
    let ingest_source = ConfiguredSource::new("input:test:bound").unwrap();
    let ingest = SourceBoundIngest::new(ingest_source.clone(), client);

    let error = AdapterStartContext::try_new(
        AdapterInstanceId::new("test_one").unwrap(),
        configured_source.clone(),
        ingest,
    )
    .expect_err("a context must not contain two different sources");

    assert_eq!(error.configured_source(), &configured_source);
    assert_eq!(error.ingest_source(), &ingest_source);
}

#[test]
fn diagnostics_bound_redact_and_validate_adapter_namespaced_codes() {
    let long_message = "界".repeat(MAX_DIAGNOSTIC_MESSAGE_BYTES);
    let diagnostic = AdapterDiagnostic::new(DiagnosticKind::Decode, long_message);
    assert!(diagnostic.message.len() <= MAX_DIAGNOSTIC_MESSAGE_BYTES);
    assert!(
        diagnostic
            .message
            .is_char_boundary(diagnostic.message.len())
    );

    let secret = AdapterDiagnostic::new(
        DiagnosticKind::Transport,
        "authorization: Bearer super-secret-token",
    );
    assert_eq!(secret.message, REDACTED_DIAGNOSTIC_MESSAGE);
    assert!(!format!("{secret:?}").contains("super-secret-token"));
    let token = AdapterDiagnostic::new(
        DiagnosticKind::Transport,
        "upstream token=ikd_test_sentinel",
    );
    assert_eq!(token.message, REDACTED_DIAGNOSTIC_MESSAGE);
    assert!(!format!("{token:?}").contains("ikd_test_sentinel"));

    let valid = AdapterDiagnostic::new(DiagnosticKind::Protocol, "bad frame")
        .with_code("rpi-local:bad-frame")
        .expect("type-namespaced code");
    assert_eq!(valid.code.as_deref(), Some("rpi-local:bad-frame"));

    assert!(
        AdapterDiagnostic::new(DiagnosticKind::Protocol, "bad frame")
            .with_code("Bad Frame")
            .is_err()
    );
    assert!(
        AdapterDiagnostic::new(DiagnosticKind::Protocol, "bad frame")
            .with_code("rpi-local")
            .is_err()
    );
}

#[tokio::test]
async fn reporter_sanitizes_public_diagnostic_fields_before_delivery() {
    let id = AdapterInstanceId::new("test_one").unwrap();
    let (runtime, mut running) = runtime_channels(id, 1);
    runtime
        .diagnostics
        .try_emit(AdapterDiagnostic {
            kind: DiagnosticKind::Transport,
            code: Some("not namespaced".repeat(32)),
            message: "password=hunter2".repeat(MAX_DIAGNOSTIC_MESSAGE_BYTES),
        })
        .unwrap();

    let diagnostic = running.diagnostics.recv().await.unwrap();
    assert_eq!(diagnostic.code, None);
    assert_eq!(diagnostic.message, REDACTED_DIAGNOSTIC_MESSAGE);
}
