//! SqliteRegistry: D6決定6(受理判別表)の評価器。受理トランザクション内で呼ばれる。
use crate::catalog::{ChannelMode, ValueType, standard_catalog};
use crate::store::{self, EntryRow, Resolution};
use iotkit_core_collector::{RegistryPolicy, RegistryVerdict};
use iotkit_core_ledger as ledger;
use iotkit_ingest_contract::{QuarantineReason, ReadingItem, ReasonCode};

/// 現場レジストリ(SQLite)を参照するRegistryPolicy本実装。状態はすべてDBにあり、
/// この構造体自体はステートレス(Arcで共有可)。
pub struct SqliteRegistry;

impl RegistryPolicy for SqliteRegistry {
    fn evaluate(
        &self,
        conn: &rusqlite::Connection,
        system_id: &ledger::SystemId,
        item: &ReadingItem,
    ) -> Result<RegistryVerdict, String> {
        evaluate_item(conn, system_id, item)
    }
}

fn evaluate_item(
    conn: &rusqlite::Connection,
    system_id: &ledger::SystemId,
    item: &ReadingItem,
) -> Result<RegistryVerdict, String> {
    let raw_channel: i32 = item
        .channel_index
        .map(i32::from)
        .unwrap_or(ledger::CHANNEL_NA);
    // 1) 解決: entries → aliases(series_key不変規則) → カタログauto-enable → 未知
    let declared = item.measurement_key.as_str();
    let (entry, resolved_key): (EntryRow, String) =
        match store::find_resolution(conn, declared).map_err(|e| e.to_string())? {
            Some(Resolution::Entry(e)) => (e, declared.to_string()),
            Some(Resolution::Alias { canonical, .. }) => {
                if standard_catalog().find(declared).is_some() {
                    // D6決定3: カタログキーと同名の現場エイリアスは自動有効化を遮蔽する。
                    // 明示解決(R14)が要る状態——Wave 0はwarnログで可視化する。
                    tracing::warn!(
                        key = declared,
                        "catalog key shadowed by location alias; explicit resolution required (D6)"
                    );
                }
                let materialized = ledger::series_exists_for_key(conn, system_id, declared)
                    .map_err(|e| e.to_string())?;
                if materialized {
                    // D6決定3(a): 実体化済み申告キーは不変。検証はcanonical定義で行う
                    // (series級検疫の解除はdefine_alias確立時に済んでいる=Task 3)
                    (canonical, declared.to_string())
                } else {
                    // D6決定3(b): 未実体化はcanonicalとして実体化
                    let key = canonical.measurement_key.clone();
                    (canonical, key)
                }
            }
            None => match standard_catalog().find(declared) {
                Some(cat_entry) => {
                    // D6決定4: カタログ内キーの初観測は自動有効化+監査イベント必須。
                    // ストレージ失敗はErrのまま上へ(ackなし=D1)——RejectItemに変換しない。
                    let e = store::enable_entry(
                        conn,
                        cat_entry,
                        &standard_catalog().catalog_version,
                        "auto",
                    )
                    .map_err(|e| e.to_string())?;
                    (e, declared.to_string())
                }
                None => {
                    // 文法適合の未知キー → 検疫(D6決定6)。定義がないため以降の検査は行わない
                    return Ok(RegistryVerdict::Accept {
                        resolved_key: declared.to_string(),
                        channel_index: raw_channel,
                        quarantine: Some(QuarantineReason::UnknownKey),
                    });
                }
            },
        };

    // 2) 値型検査(構造的に解釈不能=終端Rejected、D6決定6)
    if entry.value_type == ValueType::Record {
        return Ok(RegistryVerdict::RejectItem {
            reason_code: ReasonCode::ValueTypeMismatch,
            message: format!(
                "'{}' is a record type: wire representation is reserved for a future contract addendum (D6)",
                entry.measurement_key
            ),
        });
    }
    if item.values.len() != 1 {
        return Ok(RegistryVerdict::RejectItem {
            reason_code: ReasonCode::ValueTypeMismatch,
            message: format!(
                "scalar measurement expects exactly 1 value, got {} (multi-channel data must be split into per-channel items)",
                item.values.len()
            ),
        });
    }
    let value = item.values[0];
    if !value.is_finite() {
        // NaN/Infは値域比較を素通りする。決定的に解釈不能=終端拒否(D1)。
        // 素通りさせるとinsert_reading_v3の非有限チェックで失敗→ackなし→恒久再送ループになる。
        return Ok(RegistryVerdict::RejectItem {
            reason_code: ReasonCode::ValueTypeMismatch,
            message: format!("non-finite value {value} is structurally uninterpretable"),
        });
    }
    match entry.value_type {
        ValueType::Bool if value != 0.0 && value != 1.0 => {
            return Ok(RegistryVerdict::RejectItem {
                reason_code: ReasonCode::ValueTypeMismatch,
                message: format!("bool measurement expects 0 or 1, got {value}"),
            });
        }
        ValueType::Int if value.fract() != 0.0 => {
            return Ok(RegistryVerdict::RejectItem {
                reason_code: ReasonCode::ValueTypeMismatch,
                message: format!("int measurement expects an integral value, got {value}"),
            });
        }
        _ => {}
    }

    let variant = item
        .series_variant
        .as_deref()
        .unwrap_or(ledger::DEFAULT_VARIANT);

    // 3) チャネル検査+正準化(D6決定6/12)。single modeのNone/Some(0)は
    //    既存seriesのchannel形を尊重しつつ、新規は番兵-1へ寄せる。
    let (channel, undeclared_channel) = match entry.channel_mode {
        ChannelMode::Single => match item.channel_index {
            None | Some(0) => {
                let na_meta = ledger::find_series_meta(
                    conn,
                    system_id,
                    &resolved_key,
                    ledger::CHANNEL_NA,
                    variant,
                )
                .map_err(|e| e.to_string())?;
                let zero_meta =
                    ledger::find_series_meta(conn, system_id, &resolved_key, 0, variant)
                        .map_err(|e| e.to_string())?;
                if na_meta.is_some() {
                    if zero_meta.is_some() {
                        let detail = serde_json::json!({
                            "measurement_key": &resolved_key,
                            "variant": variant,
                            "preferred_channel": ledger::CHANNEL_NA,
                            "legacy_channel": 0,
                        })
                        .to_string();
                        ledger::record_event(
                            conn,
                            "channel_form_conflict",
                            Some(system_id),
                            &detail,
                        )
                        .map_err(|e| e.to_string())?;
                    }
                    (ledger::CHANNEL_NA, false)
                } else if zero_meta.is_some() {
                    (0, false)
                } else {
                    (ledger::CHANNEL_NA, false)
                }
            }
            Some(_) => (raw_channel, true),
        },
        ChannelMode::Fixed => match item.channel_index {
            Some(i) if (i as usize) < entry.channel_roles.len() => (raw_channel, false),
            _ => (raw_channel, true), // 範囲外もNone(帰属不能)も宣言外
        },
        ChannelMode::Generic => (raw_channel, false), // 宣言照合はWave 1(能力宣言=キュー5)
    };
    if undeclared_channel {
        return Ok(RegistryVerdict::Accept {
            resolved_key,
            channel_index: channel,
            quarantine: Some(QuarantineReason::UndeclaredChannel),
        });
    }

    // 4) 値域検査: min/max各辺独立に series個別 → エントリ現場既定 → カタログ物理限界を
    //    フォールバック(D6決定7外殻不変則: 片辺のみのseries上書きでも反対辺は外殻が生きる)
    let series_meta = ledger::find_series_meta(conn, system_id, &resolved_key, channel, variant)
        .map_err(|e| e.to_string())?;
    let series_min = series_meta.as_ref().and_then(|m| m.range_min);
    let series_max = series_meta.as_ref().and_then(|m| m.range_max);
    let min = series_min.or(entry.local_min).or(entry.physical_min);
    let max = series_max.or(entry.local_max).or(entry.physical_max);
    let out_of_range = min.is_some_and(|lo| value < lo) || max.is_some_and(|hi| value > hi);
    if out_of_range {
        return Ok(RegistryVerdict::Accept {
            resolved_key,
            channel_index: channel,
            quarantine: Some(QuarantineReason::OutOfRange),
        });
    }

    Ok(RegistryVerdict::Accept {
        resolved_key,
        channel_index: channel,
        quarantine: None,
    })
}

#[cfg(test)]
#[path = "../tests/unit/policy_tests.rs"]
mod tests;
