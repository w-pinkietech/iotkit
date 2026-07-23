use super::*;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[tokio::test]
async fn critical_task_failure_is_a_process_failure() {
    let mut supervisor = Supervisor::new();
    supervisor.spawn("mqtt-ingest", async {
        Err(CriticalTaskError::new("mqtt-ingest"))
    });

    assert_eq!(
        supervisor.run().await,
        ExitReason::CriticalTaskFailed {
            task: "mqtt-ingest"
        }
    );
}

#[tokio::test]
async fn explicit_shutdown_is_distinct_from_failure() {
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let supervisor = Supervisor::with_token(cancellation, Duration::from_secs(1));

    assert_eq!(supervisor.run().await, ExitReason::Requested);
}

#[tokio::test]
async fn clean_critical_task_exit_is_a_process_failure_and_cancels_siblings() {
    let cancellation = CancellationToken::new();
    let sibling_cancelled = Arc::new(AtomicBool::new(false));
    let mut supervisor = Supervisor::with_token(cancellation.clone(), Duration::from_secs(1));
    supervisor.spawn("http", async { Ok(()) });
    supervisor.spawn("mqtt-ingest", {
        let sibling_cancelled = sibling_cancelled.clone();
        async move {
            cancellation.cancelled().await;
            sibling_cancelled.store(true, Ordering::SeqCst);
            Ok(())
        }
    });

    assert_eq!(
        supervisor.run().await,
        ExitReason::CriticalTaskFailed { task: "http" }
    );
    assert!(sibling_cancelled.load(Ordering::SeqCst));
}

#[tokio::test]
async fn panic_is_a_process_failure() {
    let mut supervisor = Supervisor::new();
    supervisor.spawn("http", async {
        panic!("boom");
        #[allow(unreachable_code)]
        Ok(())
    });

    assert_eq!(
        supervisor.run().await,
        ExitReason::CriticalTaskFailed {
            task: "panicked-task"
        }
    );
}

#[tokio::test]
async fn uncooperative_task_makes_shutdown_time_out() {
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let mut supervisor = Supervisor::with_token(cancellation, Duration::from_millis(10));
    supervisor.spawn("stuck", std::future::pending());

    assert_eq!(supervisor.run().await, ExitReason::ShutdownTimedOut);
}
