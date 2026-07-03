use iotkit_core_ledger::{SystemId, CHANNEL_NA};
use iotkit_ingest_contract::{QuarantineReason, ReadingItem, ReasonCode};

/// series行にも検疫マークを付ける理由か(D6決定6)。
/// UnknownKey/UndeclaredChannelはseries実体そのものが疑わしい。
/// OutOfRangeはseriesは健全で観測だけが外れ値、DeviceQuarantinedはデバイス状態由来なのでfalse。
pub fn is_series_level(reason: QuarantineReason) -> bool {
    matches!(reason, QuarantineReason::UnknownKey | QuarantineReason::UndeclaredChannel)
}

#[derive(Debug, Clone)]
pub enum RegistryVerdict {
    Accept {
        /// エイリアス解決後のmeasurement_key(D6決定3。series実体化にはこちらを使う)
        resolved_key: String,
        /// 評価器が決めた正準チャネル(DB表現)。single modeの Some(0)→CHANNEL_NA 正規化込み
        /// (None/Some(0)で同一測定が別seriesに分裂するのを防ぐ)。コレクタは再計算しない。
        channel_index: i32,
        quarantine: Option<QuarantineReason>,
    },
    RejectItem { reason_code: ReasonCode, message: String },
}

/// 受理時のレジストリ検証フック(D6判別表)。本実装はiotkit-core-registryのSqliteRegistry。
/// Errはストレージ失敗として呼び出し元がackなしで処理する(D1)——RejectItemへの変換は禁止。
/// 評価器はDeviceQuarantinedを返さない(デバイス状態はコレクタの管轄)。
pub trait RegistryPolicy: Send + Sync {
    fn evaluate(
        &self,
        conn: &rusqlite::Connection,
        system_id: &SystemId,
        item: &ReadingItem,
    ) -> Result<RegistryVerdict, String>;
}

/// テスト用の素通し実装: 常にAccept(検疫なし・キー無変換・チャネル生写像)。
/// 文法検査は計画2以降コレクタ本体のprecheckに移った(このポリシーの仕事ではない)。
pub struct PermissiveRegistry;

impl RegistryPolicy for PermissiveRegistry {
    fn evaluate(
        &self,
        _conn: &rusqlite::Connection,
        _system_id: &SystemId,
        item: &ReadingItem,
    ) -> Result<RegistryVerdict, String> {
        Ok(RegistryVerdict::Accept {
            resolved_key: item.measurement_key.clone(),
            channel_index: item.channel_index.map(i32::from).unwrap_or(CHANNEL_NA),
            quarantine: None,
        })
    }
}
