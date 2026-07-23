use super::*;

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
    let supervisor = Supervisor::new();
    supervisor.request_shutdown();

    assert_eq!(supervisor.run().await, ExitReason::Requested);
}
