use std::future::Future;

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
}

impl Supervisor {
    #[must_use]
    pub fn new() -> Self {
        Self {
            cancellation: CancellationToken::new(),
            tasks: JoinSet::new(),
        }
    }

    pub fn spawn<F>(&mut self, task: &'static str, future: F)
    where
        F: Future<Output = Result<(), CriticalTaskError>> + Send + 'static,
    {
        self.tasks
            .spawn(async move { future.await.map_err(|_| CriticalTaskError::new(task)) });
    }

    pub fn request_shutdown(&self) {
        self.cancellation.cancel();
    }

    pub async fn run(mut self) -> ExitReason {
        if self.tasks.is_empty() {
            return ExitReason::Requested;
        }

        tokio::select! {
            () = self.cancellation.cancelled() => ExitReason::Requested,
            result = self.tasks.join_next() => match result {
                Some(Ok(Ok(()))) | None => ExitReason::Requested,
                Some(Ok(Err(error))) => ExitReason::CriticalTaskFailed { task: error.task },
                Some(Err(_)) => ExitReason::CriticalTaskFailed { task: "panicked-task" },
            },
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
