use std::{env, path::PathBuf};

use iotkit_edge::{
    application::{
        output_profiles::OutputProfiles,
        semantics::{SemanticRuleDraft, Semantics},
    },
    composition::registered_output_adapters,
    semantics::{Detector, RuleSpec, SemanticKind, TriggerMode},
    storage::{AcceptBatch, RawRecord, Storage, StorageProfile},
};
use iotkit_edge_custody_contract::DescriptorSnapshot;
use serde_json::Map;

#[tokio::main]
async fn main() {
    let mut args = env::args().skip(1);
    let profile = args.next().expect("storage profile");
    let location = args.next().expect("storage location");
    let storage = Storage::connect(match profile.as_str() {
        "embedded" => StorageProfile::Sqlite {
            path: PathBuf::from(location),
        },
        "postgres" => StorageProfile::Postgres { dsn: location },
        _ => panic!("storage profile must be embedded or postgres"),
    })
    .await
    .expect("connect fixture storage");
    storage
        .initialize_edge_identity(1_720_000_000_000)
        .await
        .expect("initialize Edge identity");
    let descriptor = DescriptorSnapshot::decode(include_bytes!(
        "../../testdata/egress/v2/descriptor-snapshot.json"
    ))
    .expect("decode descriptor fixture");
    storage
        .apply_descriptor(&descriptor, 1_720_000_000_000)
        .await
        .expect("apply descriptor");
    accept(&storage, 1, 20.5).await;
    let semantics = Semantics::new(storage.clone());
    semantics
        .create_rule(
            SemanticRuleDraft {
                edge_node_id: descriptor.edge_node_id.clone(),
                series_key: descriptor.signals[0].series_key.clone(),
                display_name: "稼働状態".into(),
                spec: RuleSpec {
                    kind: SemanticKind::Numeric,
                    detector: Detector::default(),
                    trigger: TriggerMode::None,
                },
            },
            1_720_000_000_010,
        )
        .await
        .expect("create semantic fixture");
    OutputProfiles::new(storage.clone(), registered_output_adapters())
        .activate(
            "IoTKit MQTT 出力",
            "iotkit.mqtt-json.v1",
            Map::new(),
            1_720_000_000_020,
        )
        .await
        .expect("create output fixture");
    accept(&storage, 2, 21.5).await;
    semantics
        .project_pending(10, registered_output_adapters())
        .await
        .expect("project semantic fixture");
    println!("{}", storage.edge_id().await.expect("read Edge identity"));
}

async fn accept(storage: &Storage, sequence: i64, value: f64) {
    let record = serde_json::json!({
        "family": "measurement",
        "schema_version": 1,
        "epoch": "epoch-01",
        "pub_seq": sequence,
        "series_key": "018f0000-0000-7000-8000-000000000001:contact_state:na:primary",
        "values": [value],
        "event_time": 1_720_000_000_000_i64 + sequence,
        "event_time_source": "received_at",
        "time_source": "edge_node",
        "time_quality": "unsynced",
        "received_at": 1_720_000_000_000_i64 + sequence,
        "device_time": null
    });
    storage
        .accept_batch(AcceptBatch {
            edge_node_id: "edge-node-01".into(),
            ledger_epoch: "epoch-01".into(),
            publication_id: format!("console-fixture-{sequence}"),
            received_at: 1_720_000_000_000 + sequence,
            records: vec![RawRecord::new(sequence, serde_json::to_vec(&record).unwrap()).unwrap()],
        })
        .await
        .expect("accept fixture record");
}
