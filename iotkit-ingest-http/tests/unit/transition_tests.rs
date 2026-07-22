use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, Clone, PartialEq, Eq)]
struct FakeListener(&'static str);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fault(&'static str);

impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for Fault {}

fn install_initial(state: &mut ListenerTransition<FakeListener>) {
    state
        .apply_generation(
            1,
            || Ok::<_, Fault>("tls-1"),
            Ok,
            |_| Ok(FakeListener("safe-1")),
            || Ok(()),
        )
        .unwrap();
}

#[tokio::test]
async fn checked_async_apply_drops_staged_listener_when_authority_changes() {
    #[derive(Debug)]
    struct DropListener(Arc<AtomicUsize>);
    impl Drop for DropListener {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    for fail_before_switchover in [false, true] {
        let drops = Arc::new(AtomicUsize::new(0));
        let staged_drops = drops.clone();
        let mut state = ListenerTransition::<DropListener>::default();
        let result = state
            .apply_generation_async_checked(
                1,
                || Ok::<_, Fault>(()),
                Ok,
                move |_| async move {
                    let staged = DropListener(staged_drops);
                    if fail_before_switchover {
                        Ok(staged)
                    } else {
                        Err(Fault("post_bind_authority_changed"))
                    }
                },
                move || async move {
                    if fail_before_switchover {
                        Err(Fault("pre_switchover_authority_changed"))
                    } else {
                        Ok(())
                    }
                },
            )
            .await;
        assert!(result.is_err());
        assert!(state.active().is_none());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn tls_stage_replacement_bind_and_switchover_failures_retain_last_safe_listener() {
    for phase in ["tls_stage", "tls_replace", "bind", "switchover"] {
        let mut state = ListenerTransition::default();
        install_initial(&mut state);
        let result = state.apply_generation(
            2,
            || {
                (phase != "tls_stage")
                    .then_some("tls-2")
                    .ok_or(Fault(phase))
            },
            |tls| (phase != "tls_replace").then_some(tls).ok_or(Fault(phase)),
            |_| {
                (phase != "bind")
                    .then_some(FakeListener("safe-2"))
                    .ok_or(Fault(phase))
            },
            || (phase != "switchover").then_some(()).ok_or(Fault(phase)),
        );
        assert!(result.is_err());
        assert_eq!(state.desired_generation(), 2);
        assert_eq!(state.applied_generation(), 1);
        assert_eq!(state.active(), Some(&FakeListener("safe-1")));
    }
}

#[test]
fn successful_switchover_activates_new_before_returning_old_for_drain() {
    let mut state = ListenerTransition::default();
    install_initial(&mut state);
    let old = state
        .apply_generation(
            2,
            || Ok::<_, Fault>("tls-2"),
            Ok,
            |_| Ok(FakeListener("safe-2")),
            || Ok(()),
        )
        .unwrap();
    assert_eq!(state.active(), Some(&FakeListener("safe-2")));
    assert_eq!(state.applied_generation(), 2);
    assert_eq!(old, Some(FakeListener("safe-1")));
}

#[test]
fn drain_failure_retains_active_and_restart_failure_retains_last_safe_generation() {
    let mut state = ListenerTransition::default();
    install_initial(&mut state);
    assert!(
        state
            .disable_generation(2, |_| Err(Fault("drain")))
            .is_err()
    );
    assert_eq!(state.active(), Some(&FakeListener("safe-1")));
    assert_eq!(state.applied_generation(), 1);

    assert!(
        state
            .apply_generation(
                2,
                || Ok::<_, Fault>("tls-2"),
                Ok,
                |_| Err(Fault("restart")),
                || Ok(()),
            )
            .is_err()
    );
    assert_eq!(state.active(), Some(&FakeListener("safe-1")));
    assert_eq!(state.applied_generation(), 1);
}

#[test]
fn stale_generation_cannot_replace_or_disable_a_newer_listener() {
    let mut state = ListenerTransition::default();
    install_initial(&mut state);
    assert!(matches!(
        state.apply_generation(
            1,
            || Ok::<_, Fault>(()),
            Ok,
            |_| Ok(FakeListener("unsafe")),
            || Ok(())
        ),
        Err(ApplyError::State(TransitionError::GenerationNotNew))
    ));
    assert_eq!(state.active(), Some(&FakeListener("safe-1")));
}

#[test]
fn invalidation_drains_and_allows_exact_desired_generation_to_be_restaged() {
    let mut state = ListenerTransition::default();
    install_initial(&mut state);
    let old = state.invalidate(|_| Ok::<_, Fault>(())).unwrap();
    assert_eq!(old, Some(FakeListener("safe-1")));
    assert!(state.active().is_none());
    assert_eq!(state.applied_generation(), 0);
    state
        .apply_generation(
            1,
            || Ok::<_, Fault>("tls-1"),
            Ok,
            |_| Ok(FakeListener("safe-1-restaged")),
            || Ok(()),
        )
        .unwrap();
}

#[test]
fn applied_state_publish_failure_rolls_back_to_the_held_safe_listener() {
    let mut state = ListenerTransition::default();
    install_initial(&mut state);
    let old = state
        .apply_generation(
            2,
            || Ok::<_, Fault>("tls-2"),
            Ok,
            |_| Ok(FakeListener("safe-2")),
            || Ok(()),
        )
        .unwrap();
    let failed = state.rollback_switchover(old, 1);
    assert_eq!(failed, Some(FakeListener("safe-2")));
    assert_eq!(state.active(), Some(&FakeListener("safe-1")));
    assert_eq!(state.applied_generation(), 1);
}

#[test]
fn paused_runtime_commit_methods_change_generation_only_after_publication() {
    let mut state = ListenerTransition::default();
    install_initial(&mut state);

    state.prepare_generation(2).unwrap();
    assert_eq!(state.applied_generation(), 1);
    assert_eq!(state.active(), Some(&FakeListener("safe-1")));
    state.commit_reused_generation(2).unwrap();
    assert_eq!(state.applied_generation(), 2);

    state.prepare_generation(3).unwrap();
    let old = state
        .commit_replaced_generation(3, FakeListener("safe-3"))
        .unwrap();
    assert_eq!(old, Some(FakeListener("safe-1")));
    assert_eq!(state.active(), Some(&FakeListener("safe-3")));

    state.prepare_generation(4).unwrap();
    let old = state.commit_disabled_generation(4).unwrap();
    assert_eq!(old, Some(FakeListener("safe-3")));
    assert!(state.active().is_none());
    assert_eq!(state.applied_generation(), 4);
}

#[test]
fn restart_restores_the_last_applied_listener_before_retrying_new_desired_state() {
    let mut state = ListenerTransition::default();
    state
        .restore_applied(2, 1, FakeListener("safe-1-recovered"))
        .unwrap();
    assert_eq!(state.desired_generation(), 2);
    assert_eq!(state.applied_generation(), 1);
    assert_eq!(state.active(), Some(&FakeListener("safe-1-recovered")));
    assert!(
        state
            .apply_generation(
                2,
                || Err::<&str, _>(Fault("restart")),
                Ok,
                |_| Ok(FakeListener("unsafe")),
                || Ok(()),
            )
            .is_err()
    );
    assert_eq!(state.active(), Some(&FakeListener("safe-1-recovered")));
}
