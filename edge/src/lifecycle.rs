use std::{future::Future, time::Duration};

use thiserror::Error;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitReason {
    Requested,
    CriticalTaskFailed { task: &'static str },
    ShutdownTimedOut,
}

#[derive(Debug, Error)]
#[error("critical task {task} failed")]
pub struct CriticalTaskError {
    task: &'static str,
}

impl CriticalTaskError {
    #[must_use]
    pub fn new(task: &'static str) -> Self {
        Self { task }
    }
}

pub struct Supervisor {
    cancellation: CancellationToken,
    tasks: JoinSet<Result<(), CriticalTaskError>>,
    shutdown_timeout: Duration,
}

impl Supervisor {
    #[must_use]
    pub fn new() -> Self {
        Self::with_token(CancellationToken::new(), Duration::from_secs(10))
    }

    #[must_use]
    pub fn with_token(cancellation: CancellationToken, shutdown_timeout: Duration) -> Self {
        Self {
            cancellation,
            tasks: JoinSet::new(),
            shutdown_timeout,
        }
    }

    pub fn spawn<F>(&mut self, task: &'static str, future: F)
    where
        F: Future<Output = Result<(), CriticalTaskError>> + Send + 'static,
    {
        self.tasks.spawn(async move {
            match future.await {
                Ok(()) | Err(_) => Err(CriticalTaskError::new(task)),
            }
        });
    }

    pub fn request_shutdown(&self) {
        self.cancellation.cancel();
    }

    pub async fn run(self) -> ExitReason {
        self.run_until(std::future::pending::<()>()).await
    }

    pub async fn run_until<F>(mut self, shutdown: F) -> ExitReason
    where
        F: Future<Output = ()>,
    {
        if self.tasks.is_empty() {
            return ExitReason::Requested;
        }

        let reason = tokio::select! {
            biased;
            () = self.cancellation.cancelled() => ExitReason::Requested,
            () = shutdown => ExitReason::Requested,
            result = self.tasks.join_next() => match result {
                Some(Ok(Ok(()))) | None => ExitReason::CriticalTaskFailed { task: "critical-task" },
                Some(Ok(Err(error))) => ExitReason::CriticalTaskFailed { task: error.task },
                Some(Err(_)) => ExitReason::CriticalTaskFailed { task: "panicked-task" },
            },
        };
        self.cancellation.cancel();

        let drained = tokio::time::timeout(self.shutdown_timeout, async {
            while self.tasks.join_next().await.is_some() {}
        })
        .await
        .is_ok();
        if drained {
            reason
        } else {
            self.tasks.abort_all();
            ExitReason::ShutdownTimedOut
        }
    }
}

impl Default for Supervisor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "../tests/unit/lifecycle_tests.rs"]
mod tests;
