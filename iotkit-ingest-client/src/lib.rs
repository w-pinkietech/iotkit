//! iotkit-ingest-client: 取り込み契約クライアント(D4の第3部品、北向き専用)。
//! Wave 0はinprocバインディングのみ。ワイヤ契約が規範であり、本クレートは便宜品(D4)。
//!
//! クライアントの義務(D1):
//! - ack意味論の消費: Accepted/Duplicate=完了、Rejected/ItemRejected=終端(再送しない)、
//!   ackなし(NoAck)=エンベロープ不変のままバックオフ再送
//! - envelope_idは構築時に一度だけ採番し、再送で変えない(dedupが吸収)
//! - 有界spool: 溢れは最古からドロップ+警告(Wave 0はメモリのみ=D1軽量プロファイル)
use iotkit_ingest_contract::{Envelope, ReadingItem};

pub const DEFAULT_QUEUE_CAP: usize = 256;
pub const DEFAULT_SPOOL_CAP: usize = 1024;
pub const RETRY_BACKOFF_MS: [u64; 4] = [100, 500, 2000, 5000];

/// エンベロープ採番の一箇所(プロセス内はUUIDv4可=D1)。
pub fn new_envelope(source: &str, items: Vec<ReadingItem>) -> Envelope {
    Envelope {
        envelope_id: uuid::Uuid::new_v4().to_string(),
        source: source.to_string(),
        declaration_version: None,
        items: items
            .into_iter()
            .filter(|item| !item.values.is_empty())
            .collect(),
    }
}

#[cfg(feature = "inproc")]
pub use inproc::{
    AbandonReason, DeliveryOutcome, DeliveryReceipt, EnqueuedEnvelope, IngestClient,
    IngestClientError, IngestClientEvent, IngestClientFull, QueueSubmitError, RetryHandle,
    TestEnvelopeReceiver, channel_for_test, spawn_inproc, spawn_inproc_observed,
};

#[cfg(feature = "inproc")]
mod inproc;
