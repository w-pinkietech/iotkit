pub mod application;
pub mod auth;
pub mod composition;
pub mod config;
pub mod lifecycle;
pub mod mqtt;
pub mod semantics;
pub mod storage;

use lifecycle::{ExitReason, Supervisor};

pub struct Application {
    supervisor: Supervisor,
}

impl Application {
    #[must_use]
    pub fn new() -> Self {
        Self {
            supervisor: Supervisor::new(),
        }
    }

    pub async fn run(self) -> ExitReason {
        self.supervisor.run().await
    }
}

impl Default for Application {
    fn default() -> Self {
        Self::new()
    }
}
