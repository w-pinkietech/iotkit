use super::*;

pub struct TestEnvelopeReceiver {
    pub(super) rx: mpsc::Receiver<Submission>,
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

/// Return an ingest client and a receiver without starting a collector task.
pub fn channel_for_test(cap: usize) -> (IngestClient, TestEnvelopeReceiver) {
    let (tx, rx) = mpsc::channel::<Submission>(cap);
    (IngestClient { tx }, TestEnvelopeReceiver { rx })
}
