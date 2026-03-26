use bravepi_codec::*;

fn build_uplink_frame(device_number: u64, sensor_type: u16, rssi: i8, flag: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::new();
    let payload_len = payload.len() as u16;
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(&device_number.to_le_bytes());
    frame.extend_from_slice(&sensor_type.to_le_bytes());
    frame.push(rssi as u8);
    frame.push(flag);
    frame.extend_from_slice(payload);
    frame
}

fn sensor_payload(battery: u8, count: u16, values: &[u8]) -> Vec<u8> {
    let mut p = vec![battery];
    p.extend_from_slice(&count.to_le_bytes());
    p.extend_from_slice(values);
    p
}

const DEVICE: u64 = 0x246880020140018b;

#[test]
fn decode_thermocouple_real_frame() {
    let raw: Vec<u8> = vec![
        0x07, 0x00, 0x8b, 0x01, 0x40, 0x01, 0x02, 0x80, 0x68, 0x24,
        0x05, 0x01, 0xae, 0x00, 0x64, 0x01, 0x00, 0x00, 0x80, 0xb3, 0x41,
    ];
    let mut codec = BravePiCodec::new();
    codec.feed(&raw);
    match codec.decode().expect("should decode") {
        BravePiFrame::Sensor(s) => {
            assert_eq!(s.device_number, "246880020140018b");
            assert_eq!(s.sensor_type_raw, 261);
            assert_eq!(s.battery, 100);
            assert_eq!(s.data_count, 1);
            assert_eq!(s.value_data, vec![0x00, 0x80, 0xb3, 0x41]);
        }
        other => panic!("expected Sensor, got {:?}", other),
    }
}

#[test]
fn decode_contact_input() {
    let payload = sensor_payload(90, 5, &[0x01, 0x00, 0x01, 0x00, 0x01]);
    let frame = build_uplink_frame(DEVICE, 257, -60, 0, &payload);
    let mut codec = BravePiCodec::new();
    codec.feed(&frame);
    match codec.decode().expect("should decode") {
        BravePiFrame::Sensor(s) => {
            assert_eq!(s.sensor_type_raw, 257);
            assert_eq!(s.battery, 90);
            assert_eq!(s.data_count, 5);
        }
        other => panic!("expected Sensor, got {:?}", other),
    }
}

#[test]
fn decode_config_response() {
    let mut payload = Vec::new();
    payload.extend_from_slice(&261u16.to_le_bytes());
    payload.extend_from_slice(&[1, 2, 3]);
    payload.push(9); payload.push(1); payload.push(4);
    payload.extend_from_slice(&100u16.to_le_bytes());
    payload.extend_from_slice(&10u32.to_le_bytes());
    let frame = build_uplink_frame(DEVICE, 0, -50, 0, &payload);
    let mut codec = BravePiCodec::new();
    codec.feed(&frame);
    match codec.decode().expect("should decode") {
        BravePiFrame::Config(cfg) => {
            assert_eq!(cfg.true_sensor_type, 261);
            assert_eq!(cfg.firmware_version, "1.2.3");
        }
        other => panic!("expected Config, got {:?}", other),
    }
}

#[test]
fn decode_continuation_frames() {
    let mut payload1 = vec![75];
    payload1.extend_from_slice(&2u16.to_le_bytes());
    payload1.extend_from_slice(&[0u8; 12]);
    let frame1 = build_uplink_frame(DEVICE, 262, -60, 1, &payload1);
    let frame2 = build_uplink_frame(DEVICE, 262, -60, 0, &[0u8; 12]);
    let mut codec = BravePiCodec::new();
    codec.feed(&frame1);
    assert!(codec.decode().is_none());
    codec.feed(&frame2);
    match codec.decode().expect("should decode") {
        BravePiFrame::Sensor(s) => { assert_eq!(s.sensor_type_raw, 262); }
        other => panic!("expected Sensor, got {:?}", other),
    }
}

#[test]
fn decode_multiple_frames() {
    let p1 = sensor_payload(100, 1, &22.5f32.to_le_bytes());
    let p2 = sensor_payload(95, 1, &23.0f32.to_le_bytes());
    let mut buf = build_uplink_frame(DEVICE, 261, -70, 0, &p1);
    buf.extend_from_slice(&build_uplink_frame(DEVICE, 261, -72, 0, &p2));
    let mut codec = BravePiCodec::new();
    codec.feed(&buf);
    assert!(matches!(codec.decode(), Some(BravePiFrame::Sensor(_))));
    assert!(matches!(codec.decode(), Some(BravePiFrame::Sensor(_))));
    assert!(codec.decode().is_none());
}

#[test]
fn decode_unknown_type_still_returns_frame() {
    let payload = sensor_payload(100, 1, &[0x42]);
    let frame = build_uplink_frame(DEVICE, 999, -50, 0, &payload);
    let mut codec = BravePiCodec::new();
    codec.feed(&frame);
    match codec.decode().expect("should decode") {
        BravePiFrame::Sensor(s) => { assert_eq!(s.sensor_type_raw, 999); }
        other => panic!("expected Sensor, got {:?}", other),
    }
}

#[test]
fn encode_immediate_uplink() {
    let f = BravePiCodec::encode_downlink("246880020140018b", &DownlinkCommand::ImmediateUplink { sensor_type: 261 }).unwrap();
    assert_eq!(f[0], 0x00);
    assert_eq!(f[13], 0x00);
    assert_eq!(u16::from_le_bytes([f[11], f[12]]), 261);
}

#[test]
fn encode_parameter_get() {
    let f = BravePiCodec::encode_downlink("246880020140018b", &DownlinkCommand::ParameterGet).unwrap();
    assert_eq!(f[13], 0x0D);
}

#[test]
fn encode_contact_output() {
    let f = BravePiCodec::encode_downlink("246880020140018b", &DownlinkCommand::ContactOutput { signal_mode: 1, signal_out_time: 5000 }).unwrap();
    assert_eq!(f[13], 0x11);
    assert_eq!(f[15], 1);
    assert_eq!(u16::from_le_bytes([f[16], f[17]]), 5000);
}

#[test]
fn encode_downlink_invalid_hex_returns_error() {
    let result = BravePiCodec::encode_downlink(
        "not_valid_hex",
        &DownlinkCommand::ParameterGet,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Invalid device number hex"));
}

#[test]
fn encode_downlink_valid_hex_returns_ok() {
    let result = BravePiCodec::encode_downlink(
        "246880020140018b",
        &DownlinkCommand::ParameterGet,
    );
    assert!(result.is_ok());
}

#[test]
fn decode_empty() {
    assert!(BravePiCodec::new().decode().is_none());
}

#[test]
fn decode_partial() {
    let mut codec = BravePiCodec::new();
    codec.feed(&[0x07, 0x00, 0x8b]);
    assert!(codec.decode().is_none());
}

#[test]
fn decode_rejects_oversized_frame() {
    let mut codec = BravePiCodec::new();
    // payload_len = 5000 (exceeds MAX_FRAME_SIZE of 4096)
    // frame_len = 2 + 12 + 5000 = 5014
    let payload_len: u16 = 5000;
    let mut frame = Vec::new();
    frame.extend_from_slice(&payload_len.to_le_bytes());
    // Fill enough bytes for the codec to read the header
    frame.extend(vec![0u8; 12 + 5000]);
    codec.feed(&frame);
    match codec.decode() {
        Some(BravePiFrame::DecodeError { reason, .. }) => {
            assert!(reason.contains("frame size exceeds maximum"), "reason was: {}", reason);
        }
        other => panic!("expected DecodeError, got {:?}", other),
    }
    // Buffer should be cleared — next decode returns None
    assert!(codec.decode().is_none());
}

#[test]
fn sensor_frame_clone_and_eq() {
    let frame = SensorFrame {
        device_number: "test".to_string(),
        sensor_type_raw: 261,
        rssi: -60,
        battery: 95,
        data_count: 1,
        value_data: vec![0x00],
    };
    let cloned = frame.clone();
    assert_eq!(frame, cloned);
}

#[test]
fn bravepi_frame_clone_and_eq() {
    let frame = BravePiFrame::DecodeError {
        device_number: "test".to_string(),
        sensor_type_raw: 0,
        reason: "test".to_string(),
    };
    let cloned = frame.clone();
    assert_eq!(frame, cloned);
}

#[test]
fn codec_default_works() {
    let mut codec = BravePiCodec::default();
    codec.feed(&[]);
    assert!(codec.decode().is_none());
}

#[test]
fn decode_rejects_oversized_continuation_payload() {
    let mut codec = BravePiCodec::new();
    // 継続フレーム (flag=1) を繰り返し送り、累積ペイロードが MAX_FRAME_SIZE を超えることを確認
    // 各フレームは小さい (payload=1000 bytes) ので個別フレームの上限チェックは通過する
    let chunk_size = 1000usize;
    for i in 0..5 {
        let frame = build_uplink_frame(DEVICE, 262, -60, 1, &vec![0u8; chunk_size]);
        codec.feed(&frame);
        let result = codec.decode();
        if i < 4 {
            // 累積 1000..4000 → まだ上限以下
            assert!(result.is_none(), "should be None at chunk {}, got {:?}", i, result);
        } else {
            // 累積 5000 → 上限超過で DecodeError
            match result {
                Some(BravePiFrame::DecodeError { reason, device_number, .. }) => {
                    assert!(reason.contains("continuation payload exceeds maximum"),
                        "reason was: {}", reason);
                    // device_number はフレームから取得できている
                    assert_eq!(device_number, "246880020140018b");
                }
                other => panic!("expected DecodeError at chunk {}, got {:?}", i, other),
            }
        }
    }
    // continuation はクリアされている → 次のデコードは None
    assert!(codec.decode().is_none());
}

#[test]
fn encode_downlink_device_bytes_match_wire_order() {
    // decode は u64::from_le_bytes でデバイス番号を読む。
    // encode の出力も同じ LE バイト順でなければならない。
    let f = BravePiCodec::encode_downlink(
        "246880020140018b",
        &DownlinkCommand::ParameterGet,
    ).unwrap();
    // device bytes は offset 3..11 (0: direction, 1-2: length, 3-10: device)
    let device_bytes = &f[3..11];
    // on-wire 期待値: 0x246880020140018b の LE 表現
    assert_eq!(device_bytes, &[0x8b, 0x01, 0x40, 0x01, 0x02, 0x80, 0x68, 0x24]);
}
