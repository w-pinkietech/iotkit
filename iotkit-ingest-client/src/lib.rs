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
mod inproc {
    use super::*;
    use iotkit_core_collector::{Collector, IngestPrincipal, IngestRequest, SubmitError};
    use iotkit_ingest_contract::{AckStatus, EnvelopeAck, ItemStatus};
    use std::collections::VecDeque;
    use tokio::sync::{mpsc, oneshot};

    /// 非ブロッキング投入の失敗理由。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum IngestClientError {
        /// 入力キュー満杯(呼び出し側はドロップしてよい——送信側の逆圧シグナル)。
        Full,
        /// コレクタ側が閉じており、以後の投入では回復しない。
        Closed,
    }

    pub type IngestClientFull = IngestClientError;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum IngestClientEvent {
        SpoolOverflow,
        SubmitNoAck,
    }

    /// Why a queued envelope stopped being owned by this client before a final Ack.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum AbandonReason {
        SpoolOverflow,
        ClientShutdown,
        CollectorClosed,
    }

    /// Opaque ownership token for retrying the exact immutable envelope.
    pub struct RetryHandle {
        envelope: Envelope,
    }

    impl std::fmt::Debug for RetryHandle {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("RetryHandle")
                .field("envelope_id", &self.envelope.envelope_id)
                .field("source", &self.envelope.source)
                .finish_non_exhaustive()
        }
    }

    impl RetryHandle {
        pub fn envelope_id(&self) -> &str {
            &self.envelope.envelope_id
        }

        pub fn source(&self) -> &str {
            &self.envelope.source
        }
    }

    #[derive(Debug)]
    pub enum DeliveryOutcome {
        Final(EnvelopeAck),
        AbandonedBeforeFinal {
            reason: AbandonReason,
            retry: RetryHandle,
        },
    }

    pub struct DeliveryReceipt {
        rx: oneshot::Receiver<DeliveryOutcome>,
    }

    impl std::fmt::Debug for DeliveryReceipt {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("DeliveryReceipt").finish_non_exhaustive()
        }
    }

    impl DeliveryReceipt {
        /// Wait for either the receiver's final Ack or explicit loss of client custody.
        pub async fn wait(self) -> DeliveryOutcome {
            self.rx
                .await
                .expect("ingest receipt sender dropped without resolving its outcome")
        }
    }

    #[derive(Debug)]
    pub struct EnqueuedEnvelope {
        pub envelope_id: String,
        pub delivery: DeliveryReceipt,
    }

    #[derive(Debug)]
    pub enum QueueSubmitError {
        Full(RetryHandle),
        Closed(RetryHandle),
    }

    struct Submission {
        envelope: Envelope,
        receipt: Option<oneshot::Sender<DeliveryOutcome>>,
    }

    struct SpoolEntry {
        request: IngestRequest,
        receipt: Option<oneshot::Sender<DeliveryOutcome>>,
    }

    /// アダプタランタイムが持つ送信ハンドル。非ブロッキング(ポーリングループ/イベントループを
    /// コレクタの都合で止めない)。
    #[derive(Clone)]
    pub struct IngestClient {
        tx: mpsc::Sender<Submission>,
    }

    pub struct TestEnvelopeReceiver {
        rx: mpsc::Receiver<Submission>,
    }

    impl TestEnvelopeReceiver {
        pub async fn recv(&mut self) -> Option<Envelope> {
            self.rx.recv().await.map(|submission| submission.envelope)
        }

        pub fn try_recv(&mut self) -> Result<Envelope, mpsc::error::TryRecvError> {
            self.rx.try_recv().map(|submission| submission.envelope)
        }
    }

    impl Drop for TestEnvelopeReceiver {
        fn drop(&mut self) {
            self.rx.close();
        }
    }

    impl IngestClient {
        /// Submit sender-owned wire data. The receiver-bound principal is added
        /// behind this handle and cannot be selected by the adapter.
        pub fn try_submit(&self, envelope: Envelope) -> Result<(), IngestClientError> {
            self.tx
                .try_send(Submission {
                    envelope,
                    receipt: None,
                })
                .map_err(|e| match e {
                    mpsc::error::TrySendError::Full(_) => IngestClientError::Full,
                    mpsc::error::TrySendError::Closed(_) => IngestClientError::Closed,
                })
        }

        pub fn try_submit_with_receipt(
            &self,
            envelope: Envelope,
        ) -> Result<EnqueuedEnvelope, QueueSubmitError> {
            let envelope_id = envelope.envelope_id.clone();
            let (tx, rx) = oneshot::channel();
            let submission = Submission {
                envelope,
                receipt: Some(tx),
            };
            self.tx
                .try_send(submission)
                .map(|()| EnqueuedEnvelope {
                    envelope_id,
                    delivery: DeliveryReceipt { rx },
                })
                .map_err(|error| match error {
                    mpsc::error::TrySendError::Full(submission) => {
                        QueueSubmitError::Full(RetryHandle {
                            envelope: submission.envelope,
                        })
                    }
                    mpsc::error::TrySendError::Closed(submission) => {
                        QueueSubmitError::Closed(RetryHandle {
                            envelope: submission.envelope,
                        })
                    }
                })
        }

        pub fn try_retry_with_receipt(
            &self,
            retry: RetryHandle,
        ) -> Result<EnqueuedEnvelope, QueueSubmitError> {
            self.try_submit_with_receipt(retry.envelope)
        }
    }

    /// テスト用: 実タスクなしでEnvelopeを捕捉する受け口を返す。
    pub fn channel_for_test(cap: usize) -> (IngestClient, TestEnvelopeReceiver) {
        let (tx, rx) = mpsc::channel::<Submission>(cap);
        (IngestClient { tx }, TestEnvelopeReceiver { rx })
    }

    /// inprocクライアントタスクを起動する。タスクはコレクタ死亡(Closed)で退出し、
    /// IoTKit EdgeはJoinHandleでそれを監視する(fail-fast=計画2のSubmitError分離の消費)。
    ///
    /// 設計不変則(計画レビュー裁定反映):
    /// - バックオフ待機中も入力の吸い上げ(spoolへの排出+drop-oldest)を止めない
    ///   (再送ループが入力を飢餓させると入力キュー側で最新が落ち、「最古からドロップ」が嘘になる)
    /// - NoAck/Deferredはエンベロープ不変で再送、ジッタ付きバックオフ(D1)
    pub fn spawn_inproc(
        collector: Collector,
        principal: IngestPrincipal,
        queue_cap: usize,
        spool_cap: usize,
    ) -> (IngestClient, tokio::task::JoinHandle<()>) {
        spawn_inproc_inner(collector, principal, queue_cap, spool_cap, None)
    }

    pub fn spawn_inproc_observed(
        collector: Collector,
        principal: IngestPrincipal,
        queue_cap: usize,
        spool_cap: usize,
        observer: mpsc::UnboundedSender<IngestClientEvent>,
    ) -> (IngestClient, tokio::task::JoinHandle<()>) {
        spawn_inproc_inner(collector, principal, queue_cap, spool_cap, Some(observer))
    }

    fn spawn_inproc_inner(
        collector: Collector,
        principal: IngestPrincipal,
        queue_cap: usize,
        spool_cap: usize,
        observer: Option<mpsc::UnboundedSender<IngestClientEvent>>,
    ) -> (IngestClient, tokio::task::JoinHandle<()>) {
        let (tx, mut rx) = mpsc::channel::<Submission>(queue_cap);
        let handle = tokio::spawn(async move {
            let mut spool: VecDeque<SpoolEntry> = VecDeque::new();
            let mut backoff_until: Option<tokio::time::Instant> = None;
            let mut attempt = 0usize;
            loop {
                // 1) 送信可能なら先頭を送る
                let ready = !spool.is_empty()
                    && backoff_until.is_none_or(|t| tokio::time::Instant::now() >= t);
                if ready {
                    let request = spool.front().expect("spool non-empty");
                    let envelope = &request.request.envelope;
                    match collector.submit(request.request.clone()).await {
                        Ok(ack) if matches!(ack.status, AckStatus::Deferred) => {
                            // inprocでは返らないが、将来バインディング共用のため意味論どおり
                            // 不変再試行する(D1)
                            schedule_retry(
                                &mut backoff_until,
                                &mut attempt,
                                &envelope.envelope_id,
                                "deferred",
                            );
                        }
                        Ok(ack) => {
                            log_ack(&ack.status, &envelope.envelope_id);
                            if let Some(mut completed) = spool.pop_front()
                                && let Some(receipt) = completed.receipt.take()
                            {
                                let _ = receipt.send(DeliveryOutcome::Final(ack));
                            }
                            attempt = 0;
                            backoff_until = None;
                        }
                        Err(SubmitError::NoAck) => {
                            if let Some(observer) = &observer {
                                notify(observer, IngestClientEvent::SubmitNoAck);
                            }
                            schedule_retry(
                                &mut backoff_until,
                                &mut attempt,
                                &envelope.envelope_id,
                                "no ack (storage failure)",
                            );
                        }
                        Err(SubmitError::ClockUntrusted) => {
                            if let Some(observer) = &observer {
                                notify(observer, IngestClientEvent::SubmitNoAck);
                            }
                            schedule_retry(
                                &mut backoff_until,
                                &mut attempt,
                                &envelope.envelope_id,
                                "trusted wall clock unavailable",
                            );
                        }
                        Err(SubmitError::AuthenticationStale) => {
                            // In-process principals never carry device-token proofs. If this
                            // boundary invariant is violated, retain the envelope and fail
                            // conservatively rather than manufacturing a terminal ack.
                            if let Some(observer) = &observer {
                                notify(observer, IngestClientEvent::SubmitNoAck);
                            }
                            schedule_retry(
                                &mut backoff_until,
                                &mut attempt,
                                &envelope.envelope_id,
                                "collector rejected an impossible local authentication proof",
                            );
                        }
                        Err(SubmitError::Closed) => {
                            tracing::error!(
                                spooled = spool.len(),
                                "collector closed; ingest client exiting (supervisor will fail-fast)"
                            );
                            abandon_all(&mut spool, &mut rx, AbandonReason::CollectorClosed);
                            return;
                        }
                    }
                    continue;
                }
                // 2) 待機: 入力受信(バックオフ中も排出継続)またはバックオフ満了
                if let Some(deadline) = backoff_until {
                    tokio::select! {
                        maybe = rx.recv() => match maybe {
                            Some(submission) => push_bounded(
                                &mut spool,
                                submission,
                                &principal,
                                spool_cap,
                                observer.as_ref(),
                            ),
                            None => {
                                abandon_all(
                                    &mut spool,
                                    &mut rx,
                                    AbandonReason::ClientShutdown,
                                );
                                return;
                            }
                        },
                        _ = tokio::time::sleep_until(deadline) => { backoff_until = None; }
                    }
                } else {
                    // ここに来るのはspoolが空の場合のみ
                    match rx.recv().await {
                        Some(submission) => push_bounded(
                            &mut spool,
                            submission,
                            &principal,
                            spool_cap,
                            observer.as_ref(),
                        ),
                        None => {
                            abandon_all(&mut spool, &mut rx, AbandonReason::ClientShutdown);
                            return;
                        }
                    }
                }
            }
        });
        (IngestClient { tx }, handle)
    }

    fn push_bounded(
        spool: &mut VecDeque<SpoolEntry>,
        submission: Submission,
        principal: &IngestPrincipal,
        cap: usize,
        observer: Option<&mpsc::UnboundedSender<IngestClientEvent>>,
    ) {
        if spool.len() >= cap {
            let dropped = spool.pop_front();
            tracing::warn!(
                envelope_id = dropped
                    .as_ref()
                    .map(|d| d.request.envelope.envelope_id.as_str()),
                "ingest spool overflow: dropping oldest (bounded spool, D1 lightweight profile)"
            );
            if let Some(mut dropped) = dropped
                && let Some(receipt) = dropped.receipt.take()
            {
                let retry = RetryHandle {
                    envelope: dropped.request.envelope,
                };
                let _ = receipt.send(DeliveryOutcome::AbandonedBeforeFinal {
                    reason: AbandonReason::SpoolOverflow,
                    retry,
                });
            }
            if let Some(observer) = observer {
                notify(observer, IngestClientEvent::SpoolOverflow);
            }
        }
        spool.push_back(SpoolEntry {
            request: IngestRequest {
                principal: principal.clone(),
                envelope: submission.envelope,
            },
            receipt: submission.receipt,
        });
    }

    fn notify(observer: &mpsc::UnboundedSender<IngestClientEvent>, event: IngestClientEvent) {
        let _ = observer.send(event);
    }

    /// バックオフ再送の予約。ジッタはenvelope_idバイト和から導く決定的値
    /// (乱数依存なしでD1のジッタ義務を満たす)。
    fn schedule_retry(
        backoff_until: &mut Option<tokio::time::Instant>,
        attempt: &mut usize,
        envelope_id: &str,
        why: &str,
    ) {
        let base = RETRY_BACKOFF_MS[(*attempt).min(RETRY_BACKOFF_MS.len() - 1)];
        let jitter = envelope_id
            .bytes()
            .fold(0u64, |a, b| a.wrapping_add(b as u64))
            % (base / 4 + 1);
        *attempt += 1;
        tracing::warn!(
            envelope_id,
            attempt = *attempt,
            backoff_ms = base + jitter,
            why,
            "retrying same envelope"
        );
        *backoff_until =
            Some(tokio::time::Instant::now() + std::time::Duration::from_millis(base + jitter));
    }

    fn shutdown_note(spool: &VecDeque<SpoolEntry>) {
        if !spool.is_empty() {
            tracing::warn!(
                spooled = spool.len(),
                "ingest client shutting down with unsent envelopes (memory spool, D1 lightweight profile)"
            );
        }
    }

    fn abandon_all(
        spool: &mut VecDeque<SpoolEntry>,
        rx: &mut mpsc::Receiver<Submission>,
        reason: AbandonReason,
    ) {
        // Close admission before draining so a racing sender cannot enqueue a
        // receipt after the final try_recv observes an empty queue.
        rx.close();
        while let Ok(submission) = rx.try_recv() {
            if let Some(receipt) = submission.receipt {
                let retry = RetryHandle {
                    envelope: submission.envelope,
                };
                let _ = receipt.send(DeliveryOutcome::AbandonedBeforeFinal { reason, retry });
            }
        }
        shutdown_note(spool);
        while let Some(mut entry) = spool.pop_front() {
            if let Some(receipt) = entry.receipt.take() {
                let retry = RetryHandle {
                    envelope: entry.request.envelope,
                };
                let _ = receipt.send(DeliveryOutcome::AbandonedBeforeFinal { reason, retry });
            }
        }
    }

    fn log_ack(status: &AckStatus, envelope_id: &str) {
        match status {
            AckStatus::Accepted { items } => {
                for (i, it) in items.iter().enumerate() {
                    if let ItemStatus::ItemRejected {
                        reason_code,
                        message,
                        ..
                    } = it
                    {
                        tracing::warn!(
                            envelope_id,
                            item = i,
                            ?reason_code,
                            message,
                            "item terminally rejected"
                        );
                    }
                }
            }
            AckStatus::Duplicate => {
                tracing::debug!(envelope_id, "duplicate (already durable)");
            }
            AckStatus::Rejected {
                reason_code,
                message,
                ..
            } => {
                tracing::warn!(
                    envelope_id,
                    ?reason_code,
                    message,
                    "envelope terminally rejected (not retried)"
                );
            }
            AckStatus::Deferred => {
                // プロセス内では返らない(D1)。防御的にログのみ
                tracing::error!(envelope_id, "unexpected Deferred on inproc binding");
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use iotkit_ingest_contract::TimeSource;

        fn item(values: Vec<f64>) -> ReadingItem {
            ReadingItem {
                subject_hint: Some("hw".into()),
                measurement_key: "temperature_c".into(),
                channel_index: None,
                series_variant: None,
                values,
                device_time_ms: None,
                time_source: TimeSource::Edge,
                age_ms: None,
                rssi: None,
                battery_pct: None,
            }
        }

        #[test]
        fn new_envelope_drops_empty_value_items() {
            let envelope = new_envelope("test", vec![item(vec![]), item(vec![1.0])]);
            assert_eq!(envelope.items.len(), 1);
            assert_eq!(envelope.items[0].values, vec![1.0]);
        }

        #[tokio::test]
        async fn try_submit_distinguishes_full_from_closed() {
            let (client, mut rx) = channel_for_test(1);
            client
                .try_submit(new_envelope("test", vec![item(vec![1.0])]))
                .unwrap();
            assert_eq!(
                client
                    .try_submit(new_envelope("test", vec![item(vec![2.0])]))
                    .unwrap_err(),
                IngestClientError::Full
            );
            rx.recv().await.expect("first item should remain queued");
            drop(rx);
            assert_eq!(
                client
                    .try_submit(new_envelope("test", vec![item(vec![3.0])]))
                    .unwrap_err(),
                IngestClientError::Closed
            );
        }

        #[tokio::test]
        async fn receipt_submit_returns_same_envelope_as_retry_handle_when_full_or_closed() {
            let (client, mut rx) = channel_for_test(1);
            client
                .try_submit(new_envelope("test", vec![item(vec![1.0])]))
                .unwrap();

            let full_envelope = new_envelope("test", vec![item(vec![2.0])]);
            let full_id = full_envelope.envelope_id.clone();
            let QueueSubmitError::Full(full_retry) = client
                .try_submit_with_receipt(full_envelope)
                .expect_err("second item must see the bounded queue as full")
            else {
                panic!("expected full");
            };
            assert_eq!(full_retry.envelope_id(), full_id);
            assert_eq!(full_retry.source(), "test");

            rx.recv().await.expect("first item should remain queued");
            drop(rx);

            let closed_envelope = new_envelope("test", vec![item(vec![3.0])]);
            let closed_id = closed_envelope.envelope_id.clone();
            let QueueSubmitError::Closed(closed_retry) = client
                .try_submit_with_receipt(closed_envelope)
                .expect_err("closed queue must return retry ownership")
            else {
                panic!("expected closed");
            };
            assert_eq!(closed_retry.envelope_id(), closed_id);
        }

        #[test]
        fn abandonment_closes_front_door_before_draining_receipts() {
            let (client, mut rx) = channel_for_test(1);
            let mut spool = VecDeque::new();

            abandon_all(&mut spool, &mut rx.rx, AbandonReason::CollectorClosed);

            let envelope = new_envelope("test", vec![item(vec![1.0])]);
            let envelope_id = envelope.envelope_id.clone();
            let QueueSubmitError::Closed(retry) = client
                .try_submit_with_receipt(envelope)
                .expect_err("abandonment must close admission before its final drain")
            else {
                panic!("expected closed");
            };
            assert_eq!(retry.envelope_id(), envelope_id);
        }
    }
}
