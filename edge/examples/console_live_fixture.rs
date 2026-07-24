//! Continuously publishes the Console demo's patrol-lamp signal.
//!
//! This fixture is intentionally outside the production binary. It makes the
//! preview visibly live while exercising the same MQTT custody path as an
//! actual Edge Node.

use std::{
    env, fs,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use iotkit_edge_custody_contract::{RecordBatch, publication_id};
use rumqttc::{AsyncClient, MqttOptions, QoS};
use serde::Deserialize;
use serde_json::value::to_raw_value;
use sqlx::postgres::PgPoolOptions;

const EDGE_NODE_ID: &str = "edge-node-01";
const LEDGER_EPOCH: &str = "epoch-01";
const SERIES_KEY: &str = "018f0000-0000-7000-8000-000000000002:illuminance_lux:na:primary";
const VALUES: &[f64] = &[145.0, 151.0, 148.0, 156.0, 680.0, 702.0, 691.0, 710.0];

#[derive(Deserialize)]
struct PostgresConfig {
    dsn: String,
}

#[tokio::main]
async fn main() {
    let mut args = env::args().skip(1);
    let postgres_config = args.next().expect("PostgreSQL config path");
    let broker_host = args.next().expect("MQTT broker host");
    let broker_port = args
        .next()
        .expect("MQTT broker port")
        .parse::<u16>()
        .expect("MQTT broker port must be an integer");
    let username = args.next().expect("MQTT username");
    let password_file = args.next().expect("MQTT password file");

    let dsn = serde_json::from_slice::<PostgresConfig>(
        &fs::read(postgres_config).expect("read PostgreSQL config"),
    )
    .expect("decode PostgreSQL config")
    .dsn;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&dsn)
        .await
        .expect("connect fixture database");
    let mut sequence = sqlx::query_scalar::<_, i64>(
        "SELECT accepted_through FROM accepted_cursors \
         WHERE edge_node_id=$1 AND ledger_epoch=$2",
    )
    .bind(EDGE_NODE_ID)
    .bind(LEDGER_EPOCH)
    .fetch_one(&pool)
    .await
    .expect("read demo Edge Node cursor");
    pool.close().await;

    let password = fs::read_to_string(password_file)
        .expect("read MQTT password file")
        .trim()
        .to_owned();
    let mut mqtt = MqttOptions::new(
        format!("iotkit-console-live-fixture-{}", std::process::id()),
        broker_host,
        broker_port,
    );
    mqtt.set_credentials(username, password);
    mqtt.set_keep_alive(Duration::from_secs(15));
    let (client, mut event_loop) = AsyncClient::new(mqtt, 16);
    tokio::spawn(async move {
        loop {
            if let Err(error) = event_loop.poll().await {
                eprintln!("MQTT fixture connection: {error}");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    });

    let mut value_index = 0_usize;
    loop {
        sequence += 1;
        let received_at = now_millis();
        let record = serde_json::json!({
            "family": "measurement",
            "schema_version": 1,
            "epoch": LEDGER_EPOCH,
            "pub_seq": sequence,
            "series_key": SERIES_KEY,
            "values": [VALUES[value_index]],
            "event_time": received_at,
            "event_time_source": "received_at",
            "time_source": "edge_node",
            "time_quality": "unsynced",
            "received_at": received_at,
            "device_time": null
        });
        let batch = RecordBatch {
            schema_version: 1,
            edge_node_id: EDGE_NODE_ID.into(),
            ledger_epoch: LEDGER_EPOCH.into(),
            publication_id: publication_id(EDGE_NODE_ID, LEDGER_EPOCH, sequence, sequence),
            cursor_start: sequence,
            cursor_end: sequence,
            records: vec![to_raw_value(&record).expect("encode fixture record")],
        };
        client
            .publish(
                format!("iotkit/v1/edge-nodes/{EDGE_NODE_ID}/records"),
                QoS::AtLeastOnce,
                false,
                serde_json::to_vec(&batch).expect("encode fixture batch"),
            )
            .await
            .expect("publish fixture batch");
        value_index = (value_index + 1) % VALUES.len();
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("fixture clock must be after Unix epoch")
        .as_millis() as i64
}
