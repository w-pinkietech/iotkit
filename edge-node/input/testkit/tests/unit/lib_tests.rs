use super::*;
use iotkit_ingest_client::channel_for_test;
use iotkit_input_adapter_host_api::{
    AdapterCompletion, AdapterDiagnostic, DiagnosticKind, SourceBoundIngest, runtime_channels,
};

#[tokio::test]
async fn reference_adapter_proves_source_binding_and_two_subject_two_kind_support() {
    let reference = ReferenceAdapter::new();
    let items = reference.observations();
    assert_finite_items(&items);
    let (client, mut receiver) = channel_for_test(1);
    let ingest = SourceBoundIngest::new(reference.source.clone(), client);
    let enqueued = ingest.try_submit(items).unwrap();
    let envelope = receiver.recv().await.unwrap();
    assert_eq!(envelope.envelope_id, enqueued.envelope_id);
    assert_eq!(envelope.source, "input:reference:one");
    assert_eq!(
        envelope.items[0].subject_hint.as_deref(),
        Some("input:reference:one:subject:a")
    );
    assert_eq!(
        envelope.items[1].subject_hint.as_deref(),
        Some("input:reference:one:subject:b")
    );
    assert_ne!(
        envelope.items[0].measurement_key,
        envelope.items[1].measurement_key
    );
    assert_standard_registry_mapping(&envelope.items[..1], "temperature_c", Some("Cel"));
    assert_standard_registry_mapping(&envelope.items[1..], "contact_state", Some("1"));
}

#[tokio::test]
async fn generic_runtime_channels_cover_activity_diagnostics_shutdown_and_completion() {
    let id = AdapterInstanceId::new("reference_lifecycle").unwrap();
    let (mut runtime, mut running) = runtime_channels(id, 1);
    runtime.activity.physical_decode();
    runtime.activity.queue_admission();
    assert!(running.activity.snapshot().last_physical_decode.is_some());
    assert!(running.activity.snapshot().last_queue_admission.is_some());

    runtime
        .diagnostics
        .try_emit(AdapterDiagnostic::new(
            DiagnosticKind::Transport,
            "password=must-not-escape",
        ))
        .unwrap();
    let diagnostic = running.diagnostics.recv().await.unwrap();
    assert_eq!(diagnostic.message, "[redacted]");

    running.shutdown.request();
    assert!(runtime.stop.changed().await);
    runtime
        .completion
        .complete(AdapterCompletion::RequestedStop);
    assert_eq!(
        running.completion.wait().await,
        AdapterCompletion::RequestedStop
    );
}

#[tokio::test]
async fn reference_adapter_exercises_descriptor_config_start_and_shutdown() {
    let reference = ReferenceAdapter::new();
    assert_descriptor_v1(&ReferenceAdapter::descriptor());
    assert!(
        ReferenceAdapter::parse_and_validate(ReferenceAdapterConfig {
            diagnostic_capacity: 0,
        })
        .is_err()
    );
    let config = ReferenceAdapter::parse_and_validate(ReferenceAdapterConfig {
        diagnostic_capacity: 1,
    })
    .unwrap();

    let (client, mut receiver) = channel_for_test(1);
    let ingest = SourceBoundIngest::new(reference.source.clone(), client);
    let context = iotkit_input_adapter_host_api::AdapterStartContext::new(
        reference.instance_id.clone(),
        reference.source.clone(),
        ingest,
    );
    let running = reference.start(context, config).unwrap();

    let envelope = receiver.recv().await.unwrap();
    assert_eq!(envelope.source, reference.source.as_str());
    assert_eq!(envelope.items, reference.observations());
    assert!(running.activity.snapshot().last_physical_decode.is_some());
    assert!(running.activity.snapshot().last_queue_admission.is_some());

    running.shutdown.request();
    assert_eq!(
        running.completion.wait().await,
        AdapterCompletion::RequestedStop
    );
}
