//! BravePI UART プロトコル codec。
//! フレームをバイト列から分解するだけ。sensor の知識は持たない。

/// デコード済みフレーム。
#[derive(Debug)]
pub enum BravePiFrame {
    /// センサーデータ (sensor_type != 0)
    Sensor(SensorFrame),
    /// 設定レスポンス (sensor_type == 0)
    Config(ConfigFrame),
    /// デコードエラー
    DecodeError {
        device_number: String,
        sensor_type_raw: u16,
        reason: String,
    },
}

/// センサーデータフレーム。値の解釈は呼び出し元の責務。
#[derive(Debug)]
pub struct SensorFrame {
    pub device_number: String,
    pub sensor_type_raw: u16,
    pub rssi: i8,
    pub battery: u8,
    pub data_count: u16,
    pub value_data: Vec<u8>,
}

/// 設定レスポンスフレーム。
#[derive(Debug)]
pub struct ConfigFrame {
    pub device_number: String,
    pub rssi: i8,
    pub true_sensor_type: u16,
    pub firmware_version: String,
    pub timezone: u8,
    pub ble_mode: u8,
    pub tx_power: u8,
    pub advertise_interval: u16,
    pub uplink_interval: u32,
}

/// Downlink コマンド。
#[derive(Debug)]
pub enum DownlinkCommand {
    ImmediateUplink { sensor_type: u16 },
    ParameterGet,
    ContactOutput { signal_mode: u8, signal_out_time: u16 },
}

// ============================================================
// Codec
// ============================================================

const POST_LENGTH_HEADER: usize = 12;
const HEADER_SIZE: usize = 2 + POST_LENGTH_HEADER;
const MAX_FRAME_SIZE: usize = 4096;

pub struct BravePiCodec {
    buf: Vec<u8>,
    continuation: Option<ContinuationState>,
}

struct ContinuationState {
    device_number: String,
    sensor_type_raw: u16,
    rssi: i8,
    payload: Vec<u8>,
}

impl BravePiCodec {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(4096),
            continuation: None,
        }
    }

    pub fn feed(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    pub fn decode(&mut self) -> Option<BravePiFrame> {
        loop {
            if self.buf.len() < 2 {
                return None;
            }

            let payload_len = u16::from_le_bytes([self.buf[0], self.buf[1]]) as usize;
            let frame_len = 2 + POST_LENGTH_HEADER + payload_len;

            // フレームサイズ上限チェック
            if frame_len > MAX_FRAME_SIZE {
                self.buf.clear();
                self.continuation = None;
                return Some(BravePiFrame::DecodeError {
                    device_number: "unknown".to_string(),
                    sensor_type_raw: 0,
                    reason: format!(
                        "frame size exceeds maximum: {} > {}",
                        frame_len, MAX_FRAME_SIZE
                    ),
                });
            }

            if self.buf.len() < frame_len {
                return None;
            }

            let frame: Vec<u8> = self.buf.drain(..frame_len).collect();

            let device_number = format!(
                "{:016x}",
                u64::from_le_bytes([
                    frame[2], frame[3], frame[4], frame[5],
                    frame[6], frame[7], frame[8], frame[9],
                ])
            );
            let sensor_type_raw = u16::from_le_bytes([frame[10], frame[11]]);
            let rssi = frame[12] as i8;
            let flag = frame[13];
            let payload = &frame[HEADER_SIZE..];

            if flag == 1 {
                match &mut self.continuation {
                    Some(cont) => cont.payload.extend_from_slice(payload),
                    None => {
                        self.continuation = Some(ContinuationState {
                            device_number,
                            sensor_type_raw,
                            rssi,
                            payload: payload.to_vec(),
                        });
                    }
                }
                continue;
            }

            let (device_number, sensor_type_raw, rssi, full_payload) =
                if let Some(mut cont) = self.continuation.take() {
                    cont.payload.extend_from_slice(payload);
                    (cont.device_number, cont.sensor_type_raw, cont.rssi, cont.payload)
                } else {
                    (device_number, sensor_type_raw, rssi, payload.to_vec())
                };

            if sensor_type_raw == 0 {
                return Some(decode_config(&device_number, rssi, &full_payload));
            } else {
                return Some(decode_sensor(&device_number, sensor_type_raw, rssi, &full_payload));
            }
        }
    }

    pub fn encode_downlink(device_number_hex: &str, cmd: &DownlinkCommand) -> Vec<u8> {
        let device_bytes = Self::hex_to_device_bytes(device_number_hex);

        let (opcode, cmd_data, sensor_type_bytes) = match cmd {
            DownlinkCommand::ImmediateUplink { sensor_type } => {
                (0x00u8, vec![], sensor_type.to_le_bytes())
            }
            DownlinkCommand::ParameterGet => {
                (0x0D, vec![0x00], [0x00, 0x00])
            }
            DownlinkCommand::ContactOutput { signal_mode, signal_out_time } => {
                let mut data = vec![*signal_mode];
                data.extend_from_slice(&signal_out_time.to_le_bytes());
                (0x11, data, [0x00, 0x00])
            }
        };

        let length = (12 + cmd_data.len()) as u16;
        let mut frame = Vec::new();
        frame.push(0x00);
        frame.extend_from_slice(&length.to_le_bytes());
        frame.extend_from_slice(&device_bytes);
        frame.extend_from_slice(&sensor_type_bytes);
        frame.push(opcode);
        frame.push(0x00);
        frame.extend_from_slice(&cmd_data);
        frame
    }

    fn hex_to_device_bytes(hex: &str) -> [u8; 8] {
        let val = u64::from_str_radix(hex, 16).unwrap_or(0);
        let le = val.to_le_bytes();
        [le[7], le[6], le[5], le[4], le[3], le[2], le[1], le[0]]
    }
}

// ============================================================
// Frame decoders
// ============================================================

fn decode_config(device_number: &str, rssi: i8, payload: &[u8]) -> BravePiFrame {
    if payload.len() < 14 {
        return BravePiFrame::DecodeError {
            device_number: device_number.to_string(),
            sensor_type_raw: 0,
            reason: format!("config payload too short: {} bytes", payload.len()),
        };
    }

    BravePiFrame::Config(ConfigFrame {
        device_number: device_number.to_string(),
        rssi,
        true_sensor_type: u16::from_le_bytes([payload[0], payload[1]]),
        firmware_version: format!("{}.{}.{}", payload[2], payload[3], payload[4]),
        timezone: payload[5],
        ble_mode: payload[6],
        tx_power: payload[7],
        advertise_interval: u16::from_le_bytes([payload[8], payload[9]]),
        uplink_interval: u32::from_le_bytes([payload[10], payload[11], payload[12], payload[13]]),
    })
}

fn decode_sensor(device_number: &str, sensor_type_raw: u16, rssi: i8, payload: &[u8]) -> BravePiFrame {
    if payload.len() < 3 {
        return BravePiFrame::DecodeError {
            device_number: device_number.to_string(),
            sensor_type_raw,
            reason: format!("sensor payload too short: {} bytes", payload.len()),
        };
    }

    let battery = payload[0];
    let data_count = u16::from_le_bytes([payload[1], payload[2]]);
    let value_data = payload[3..].to_vec();

    BravePiFrame::Sensor(SensorFrame {
        device_number: device_number.to_string(),
        sensor_type_raw,
        rssi,
        battery,
        data_count,
        value_data,
    })
}
