use super::*;

const FIXTURE_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../testdata/observation/v1"
);

struct Fixture {
    name: String,
    form: String,
    topic: String,
    payload: String,
}

fn status_fixtures() -> Vec<Fixture> {
    let mut fixtures: Vec<Fixture> = std::fs::read_dir(FIXTURE_DIR)
        .expect("fixture directory")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let name = path.file_name()?.to_str()?.to_owned();
            if !name.starts_with("status-") || !name.ends_with(".json") {
                return None;
            }
            let value: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
            Some(Fixture {
                name,
                form: value["status_form"].as_str()?.to_owned(),
                topic: value["topic"].as_str()?.to_owned(),
                payload: value["payload"].as_str()?.to_owned(),
            })
        })
        .collect();
    fixtures.sort_by(|a, b| a.name.cmp(&b.name));
    assert!(
        fixtures.len() >= 6,
        "expected the contract status fixtures, found {}",
        fixtures.len()
    );
    fixtures
}

fn fault_from_fixture(value: &serde_json::Value) -> Fault {
    let since = InputTime {
        uptime_ms: value["since_uptime_ms"].as_i64().unwrap(),
        unix_epoch_ms: value["since_unix_epoch_ms"].as_i64(),
    };
    let detail = value["detail"].as_str().map(str::to_owned);
    let kind = match value["kind"].as_str().unwrap() {
        "storage-write-failed" => FaultKind::StorageWriteFailed {
            count: value["count"].as_u64().unwrap(),
        },
        "interface-open-failed" => FaultKind::InterfaceOpenFailed {
            adapter: value["adapter"].as_str().unwrap().to_owned(),
            reason: match value["reason"].as_str().unwrap() {
                "not-found" => InterfaceOpenReason::NotFound,
                "permission-denied" => InterfaceOpenReason::PermissionDenied,
                "busy" => InterfaceOpenReason::Busy,
                "io-error" => InterfaceOpenReason::IoError,
                other => panic!("unknown reason {other}"),
            },
        },
        other => panic!("unknown fault kind {other}"),
    };
    Fault {
        kind,
        since,
        detail,
    }
}

#[test]
fn producer_status_matches_every_contract_fixture_byte_for_byte() {
    for fixture in status_fixtures() {
        let parts: Vec<&str> = fixture.topic.split('/').collect();
        assert_eq!(
            parts[..3],
            ["iotkit", "v1", "edge-node"],
            "{}",
            fixture.name
        );
        assert_eq!(parts[4], "status", "{}", fixture.name);
        let edge_node_id: EdgeNodeId = parts[3].parse().unwrap();
        assert_eq!(
            status_topic(&edge_node_id),
            fixture.topic,
            "{}",
            fixture.name
        );

        if fixture.form == "offline-will" {
            assert_eq!(WILL_PAYLOAD, fixture.payload.as_bytes(), "{}", fixture.name);
            continue;
        }
        let expected: serde_json::Value = serde_json::from_str(&fixture.payload).unwrap();
        let value = match expected["value"].as_str().unwrap() {
            "online" => StatusValue::Online,
            "degraded" => StatusValue::Degraded,
            "offline" => StatusValue::Offline,
            other => panic!("{}: unknown value {other}", fixture.name),
        };
        let status = Status {
            at: InputTime {
                uptime_ms: expected["uptime_ms"].as_i64().unwrap(),
                unix_epoch_ms: expected["unix_epoch_ms"].as_i64(),
            },
            value,
            faults: expected["faults"]
                .as_array()
                .unwrap()
                .iter()
                .map(fault_from_fixture)
                .collect(),
        };
        assert_eq!(
            String::from_utf8(status.payload()).unwrap(),
            fixture.payload,
            "{}",
            fixture.name
        );
    }
}

#[test]
fn interface_open_reason_follows_the_io_error_kind() {
    use std::io::ErrorKind;
    assert_eq!(
        InterfaceOpenReason::from_io_kind(ErrorKind::NotFound),
        InterfaceOpenReason::NotFound
    );
    assert_eq!(
        InterfaceOpenReason::from_io_kind(ErrorKind::PermissionDenied),
        InterfaceOpenReason::PermissionDenied
    );
    assert_eq!(
        InterfaceOpenReason::from_io_kind(ErrorKind::ResourceBusy),
        InterfaceOpenReason::Busy
    );
    assert_eq!(
        InterfaceOpenReason::from_io_kind(ErrorKind::TimedOut),
        InterfaceOpenReason::IoError
    );
}
