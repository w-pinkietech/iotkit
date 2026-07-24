//! Exercises a clean Console commissioning journey over the MQTT product contract.
//!
//! This fixture deliberately has no storage access. It behaves like one
//! broker-enrolled Edge Node: descriptor first, activation correlation next,
//! and contiguous custody records only after activation.

use std::{
    env, fs,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use iotkit_edge_custody_contract::{
    AcceptedThrough, ActivationRequest, ActivationResult, DescriptorDevice, DescriptorSignal,
    DescriptorSnapshot, RecordBatch, publication_id,
};
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use serde_json::value::to_raw_value;

const EDGE_NODE_ID: &str = "edge-node-commissioning";
const LEDGER_EPOCH: &str = "epoch-console-commissioning-98";
const DEVICE_SYSTEM_ID: &str = "0198a4c0-0000-7000-8000-000000000001";
const SERIES_KEY: &str =
    "0198a4c0-0000-7000-8000-000000000001:steam_temperature_c:na:commissioning";
const ACTIVATION_TOPIC: &str = "iotkit/v1/edge-nodes/edge-node-commissioning/activation/request";
const RESULT_TOPIC: &str = "iotkit/v1/edge-nodes/edge-node-commissioning/activation/result";
const DESCRIPTOR_TOPIC: &str = "iotkit/v1/edge-nodes/edge-node-commissioning/descriptors";
const RECORDS_TOPIC: &str = "iotkit/v1/edge-nodes/edge-node-commissioning/records";
const ACK_TOPIC: &str = "iotkit/v1/edge-nodes/edge-node-commissioning/accepted-through";

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("commissioning fixture failed: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let broker_host = args.next().ok_or("MQTT broker host is required")?;
    let broker_port = args
        .next()
        .ok_or("MQTT broker port is required")?
        .parse::<u16>()?;
    let username = args.next().ok_or("MQTT username is required")?;
    let password_file = args.next().ok_or("MQTT password file is required")?;
    if args.next().is_some() {
        return Err("unexpected commissioning fixture argument".into());
    }
    let password = fs::read_to_string(password_file)?.trim().to_owned();

    let mut options = MqttOptions::new(
        format!("iotkit-console-commissioning-{}", std::process::id()),
        broker_host,
        broker_port,
    );
    options.set_credentials(username, password);
    options.set_keep_alive(Duration::from_secs(15));
    let (client, mut event_loop) = AsyncClient::new(options, 16);

    wait_for_connack(&mut event_loop).await?;
    client.subscribe(ACTIVATION_TOPIC, QoS::AtLeastOnce).await?;
    client.subscribe(ACK_TOPIC, QoS::AtLeastOnce).await?;
    wait_for_subscriptions(&mut event_loop, 2).await?;

    let descriptor = descriptor();
    descriptor.validate()?;
    client
        .publish(
            DESCRIPTOR_TOPIC,
            QoS::AtLeastOnce,
            true,
            serde_json::to_vec(&descriptor)?,
        )
        .await?;
    println!("commissioning descriptor published");

    let request = wait_for_activation(&mut event_loop).await?;
    println!("commissioning activation request validated");
    wait_for_matching_activation_retry(&mut event_loop, &request).await?;
    println!("commissioning activation retry validated");
    client
        .publish(
            RESULT_TOPIC,
            QoS::AtLeastOnce,
            false,
            serde_json::to_vec(&ActivationResult {
                schema_version: 1,
                activation_id: request.activation_id,
                edge_id: request.edge_id,
                edge_node_id: request.edge_node_id,
                ledger_epoch: request.expected_ledger_epoch,
                status: "applied".into(),
                discard_through_reading_seq: 0,
                first_publication_seq: 1,
                applied_at: now_millis(),
            })?,
        )
        .await?;
    println!("commissioning activation result published");

    let batch = measurement_batch()?;
    batch.validate()?;
    client
        .publish(
            RECORDS_TOPIC,
            QoS::AtLeastOnce,
            false,
            serde_json::to_vec(&batch)?,
        )
        .await?;
    println!("commissioning records 1-2 published after activation");

    let ack = wait_for_ack(&mut event_loop).await?;
    ack.validate_for(&batch, 0)?;
    println!("commissioning accepted-through 2 validated");
    client.disconnect().await?;
    Ok(())
}

fn descriptor() -> DescriptorSnapshot {
    DescriptorSnapshot {
        schema_version: 2,
        edge_node_id: EDGE_NODE_ID.into(),
        ledger_epoch: LEDGER_EPOCH.into(),
        descriptor_revision: 1,
        complete: true,
        devices: vec![DescriptorDevice {
            system_id: DEVICE_SYSTEM_ID.into(),
            identifier: Some("commissioning-98-device".into()),
            state: "active".into(),
            model_id: Some("console-commissioning-temperature".into()),
        }],
        signals: vec![DescriptorSignal {
            series_key: SERIES_KEY.into(),
            system_id: DEVICE_SYSTEM_ID.into(),
            measurement_key: "steam_temperature_c".into(),
            channel_index: None,
            variant: "commissioning".into(),
            unit: Some("Cel".into()),
            value_type: "float".into(),
        }],
    }
}

fn measurement_batch() -> Result<RecordBatch, serde_json::Error> {
    let received_at = now_millis();
    let records = [41.0, 42.5]
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let sequence = index as i64 + 1;
            to_raw_value(&serde_json::json!({
                "family": "measurement",
                "schema_version": 1,
                "epoch": LEDGER_EPOCH,
                "pub_seq": sequence,
                "series_key": SERIES_KEY,
                "values": [value],
                "event_time": received_at + sequence,
                "event_time_source": "received_at",
                "time_source": "edge_node",
                "time_quality": "unsynced",
                "received_at": received_at + sequence,
                "device_time": null
            }))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(RecordBatch {
        schema_version: 1,
        edge_node_id: EDGE_NODE_ID.into(),
        ledger_epoch: LEDGER_EPOCH.into(),
        publication_id: publication_id(EDGE_NODE_ID, LEDGER_EPOCH, 1, 2),
        cursor_start: 1,
        cursor_end: 2,
        records,
    })
}

async fn wait_for_connack(
    event_loop: &mut rumqttc::EventLoop,
) -> Result<(), Box<dyn std::error::Error>> {
    wait_for_event(event_loop, "Broker connection", |event| {
        matches!(event, Event::Incoming(Incoming::ConnAck(_)))
    })
    .await
}

async fn wait_for_subscriptions(
    event_loop: &mut rumqttc::EventLoop,
    expected: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut received = 0;
    tokio::time::timeout(Duration::from_secs(15), async {
        while received < expected {
            if matches!(
                event_loop.poll().await?,
                Event::Incoming(Incoming::SubAck(_))
            ) {
                received += 1;
            }
        }
        Ok::<_, rumqttc::ConnectionError>(())
    })
    .await
    .map_err(|_| "timed out waiting for MQTT subscriptions")??;
    Ok(())
}

async fn wait_for_activation(
    event_loop: &mut rumqttc::EventLoop,
) -> Result<ActivationRequest, Box<dyn std::error::Error>> {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Event::Incoming(Incoming::Publish(publication)) = event_loop.poll().await?
                && publication.topic == ACTIVATION_TOPIC
            {
                let request = ActivationRequest::decode(&publication.payload)?;
                if request.edge_node_id != EDGE_NODE_ID
                    || request.expected_ledger_epoch != LEDGER_EPOCH
                {
                    return Err("activation request did not match descriptor identity".into());
                }
                return Ok(request);
            }
        }
    })
    .await
    .map_err(|_| "timed out waiting for activation request")?
}

async fn wait_for_matching_activation_retry(
    event_loop: &mut rumqttc::EventLoop,
    expected: &ActivationRequest,
) -> Result<(), Box<dyn std::error::Error>> {
    let expected = serde_json::to_value(expected)?;
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Event::Incoming(Incoming::Publish(publication)) = event_loop.poll().await?
                && publication.topic == ACTIVATION_TOPIC
            {
                let request = ActivationRequest::decode(&publication.payload)?;
                if serde_json::to_value(request)? != expected {
                    return Err("activation retry changed the exact request".into());
                }
                return Ok(());
            }
        }
    })
    .await
    .map_err(|_| "timed out waiting for exact activation retry")?
}

async fn wait_for_ack(
    event_loop: &mut rumqttc::EventLoop,
) -> Result<AcceptedThrough, Box<dyn std::error::Error>> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Event::Incoming(Incoming::Publish(publication)) = event_loop.poll().await?
                && publication.topic == ACK_TOPIC
            {
                return Ok(AcceptedThrough::decode(&publication.payload)?);
            }
        }
    })
    .await
    .map_err(|_| "timed out waiting for exact accepted-through")?
}

async fn wait_for_event(
    event_loop: &mut rumqttc::EventLoop,
    description: &'static str,
    matches: impl Fn(&Event) -> bool,
) -> Result<(), Box<dyn std::error::Error>> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let event = event_loop.poll().await?;
            if matches(&event) {
                return Ok::<_, rumqttc::ConnectionError>(());
            }
        }
    })
    .await
    .map_err(|_| format!("timed out waiting for {description}"))??;
    Ok(())
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("fixture clock must be after Unix epoch")
        .as_millis() as i64
}
