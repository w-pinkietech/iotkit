use iotkit_ingest_contract::{validate_measurement_key, ReadingItem, ReasonCode};

#[derive(Debug, Clone)]
pub enum RegistryVerdict {
    Accept { quarantine: bool },
    RejectItem { reason_code: ReasonCode, message: String },
}

/// 受理時のレジストリ検証フック。計画2(D6現場レジストリ)が本実装を差し込む。
pub trait RegistryPolicy: Send + Sync {
    fn evaluate(&self, item: &ReadingItem) -> RegistryVerdict;
}

/// 計画1の暫定実装: 文法検証のみ(D6決定2)。値域・未知キー検疫は計画2で。
pub struct PermissiveRegistry;

impl RegistryPolicy for PermissiveRegistry {
    fn evaluate(&self, item: &ReadingItem) -> RegistryVerdict {
        match validate_measurement_key(&item.measurement_key) {
            Ok(()) => RegistryVerdict::Accept { quarantine: false },
            Err(e) => RegistryVerdict::RejectItem {
                reason_code: ReasonCode::MalformedMeasurementKey,
                message: e.to_string(),
            },
        }
    }
}
