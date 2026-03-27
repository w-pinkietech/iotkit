//! Transport 層の型定義。adapter crate 内部でのみ使用する。

use std::fmt;
use tokio::sync::mpsc;

/// Transport source が回復不能な障害で停止した理由。
#[derive(Debug, Clone)]
pub(crate) struct TransportError {
    pub message: String,
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// event_loop が受け取る byte stream の型。
pub(crate) type BytesReceiver = mpsc::Receiver<Result<Vec<u8>, TransportError>>;
