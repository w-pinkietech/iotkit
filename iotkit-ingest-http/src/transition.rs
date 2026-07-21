/// Generation-aware, stage-before-switchover holder used by the Edge Node composition root.
///
/// The closures make each externally fallible phase explicit. Until all four succeed the old
/// transport remains active and the applied generation does not move. The returned old transport
/// is drained only after the new safe transport has become active.
#[derive(Debug)]
pub struct ListenerTransition<T> {
    desired_generation: u64,
    applied_generation: u64,
    active: Option<T>,
}

impl<T> Default for ListenerTransition<T> {
    fn default() -> Self {
        Self {
            desired_generation: 0,
            applied_generation: 0,
            active: None,
        }
    }
}

impl<T> ListenerTransition<T> {
    pub fn desired_generation(&self) -> u64 {
        self.desired_generation
    }

    pub fn applied_generation(&self) -> u64 {
        self.applied_generation
    }

    pub fn active(&self) -> Option<&T> {
        self.active.as_ref()
    }

    pub fn restore_applied(
        &mut self,
        desired_generation: u64,
        applied_generation: u64,
        active: T,
    ) -> Result<(), TransitionError> {
        if applied_generation == 0 || applied_generation > desired_generation {
            return Err(TransitionError::GenerationRollback);
        }
        self.desired_generation = desired_generation;
        self.applied_generation = applied_generation;
        self.active = Some(active);
        Ok(())
    }

    pub fn observe_desired(&mut self, generation: u64) -> Result<(), TransitionError> {
        if generation < self.desired_generation || generation < self.applied_generation {
            return Err(TransitionError::GenerationRollback);
        }
        self.desired_generation = generation;
        Ok(())
    }

    /// Reserve a desired generation before external staging. Runtime state is unchanged until a
    /// caller has completed the durable publication boundary and invokes one of the commit
    /// methods below.
    pub fn prepare_generation(&mut self, generation: u64) -> Result<(), TransitionError> {
        self.observe_desired(generation)?;
        if generation <= self.applied_generation {
            return Err(TransitionError::GenerationNotNew);
        }
        Ok(())
    }

    /// Commits a generation whose existing listener object stayed bound while its runtime policy
    /// was paused and replaced.
    pub fn commit_reused_generation(&mut self, generation: u64) -> Result<(), TransitionError> {
        if generation != self.desired_generation || generation <= self.applied_generation {
            return Err(TransitionError::GenerationNotNew);
        }
        if self.active.is_none() {
            return Err(TransitionError::GenerationRollback);
        }
        self.applied_generation = generation;
        Ok(())
    }

    /// Commits a staged listener after durable publication and returns the prior listener for
    /// orderly drain. The staged listener must already be active to external accepts.
    pub fn commit_replaced_generation(
        &mut self,
        generation: u64,
        active: T,
    ) -> Result<Option<T>, TransitionError> {
        if generation != self.desired_generation || generation <= self.applied_generation {
            return Err(TransitionError::GenerationNotNew);
        }
        let old = self.active.replace(active);
        self.applied_generation = generation;
        Ok(old)
    }

    /// Commits a disabled generation after its prior listener has been paused and durable state
    /// names the disabled desired configuration.
    pub fn commit_disabled_generation(
        &mut self,
        generation: u64,
    ) -> Result<Option<T>, TransitionError> {
        if generation != self.desired_generation || generation < self.applied_generation {
            return Err(TransitionError::GenerationRollback);
        }
        let old = self.active.take();
        self.applied_generation = generation;
        Ok(old)
    }

    pub fn apply_generation<Tls, Prepared, E>(
        &mut self,
        generation: u64,
        stage_tls: impl FnOnce() -> Result<Tls, E>,
        replace_tls: impl FnOnce(Tls) -> Result<Prepared, E>,
        bind: impl FnOnce(Prepared) -> Result<T, E>,
        before_switchover: impl FnOnce() -> Result<(), E>,
    ) -> Result<Option<T>, ApplyError<E>> {
        self.observe_desired(generation)
            .map_err(ApplyError::State)?;
        if generation <= self.applied_generation {
            return Err(ApplyError::State(TransitionError::GenerationNotNew));
        }
        let tls = stage_tls().map_err(ApplyError::External)?;
        let prepared = replace_tls(tls).map_err(ApplyError::External)?;
        let staged = bind(prepared).map_err(ApplyError::External)?;
        before_switchover().map_err(ApplyError::External)?;

        let old = self.active.replace(staged);
        self.applied_generation = generation;
        Ok(old)
    }

    pub async fn apply_generation_async<Tls, Prepared, E, BindFuture>(
        &mut self,
        generation: u64,
        stage_tls: impl FnOnce() -> Result<Tls, E>,
        replace_tls: impl FnOnce(Tls) -> Result<Prepared, E>,
        bind: impl FnOnce(Prepared) -> BindFuture,
        before_switchover: impl FnOnce() -> Result<(), E>,
    ) -> Result<Option<T>, ApplyError<E>>
    where
        BindFuture: std::future::Future<Output = Result<T, E>>,
    {
        self.observe_desired(generation)
            .map_err(ApplyError::State)?;
        if generation <= self.applied_generation {
            return Err(ApplyError::State(TransitionError::GenerationNotNew));
        }
        let tls = stage_tls().map_err(ApplyError::External)?;
        let prepared = replace_tls(tls).map_err(ApplyError::External)?;
        let staged = bind(prepared).await.map_err(ApplyError::External)?;
        before_switchover().map_err(ApplyError::External)?;

        let old = self.active.replace(staged);
        self.applied_generation = generation;
        Ok(old)
    }

    pub async fn apply_generation_async_checked<Tls, Prepared, E, BindFuture, CheckFuture>(
        &mut self,
        generation: u64,
        stage_tls: impl FnOnce() -> Result<Tls, E>,
        replace_tls: impl FnOnce(Tls) -> Result<Prepared, E>,
        bind_and_check: impl FnOnce(Prepared) -> BindFuture,
        before_switchover: impl FnOnce() -> CheckFuture,
    ) -> Result<Option<T>, ApplyError<E>>
    where
        BindFuture: std::future::Future<Output = Result<T, E>>,
        CheckFuture: std::future::Future<Output = Result<(), E>>,
    {
        self.observe_desired(generation)
            .map_err(ApplyError::State)?;
        if generation <= self.applied_generation {
            return Err(ApplyError::State(TransitionError::GenerationNotNew));
        }
        let tls = stage_tls().map_err(ApplyError::External)?;
        let prepared = replace_tls(tls).map_err(ApplyError::External)?;
        let staged = bind_and_check(prepared)
            .await
            .map_err(ApplyError::External)?;
        before_switchover().await.map_err(ApplyError::External)?;

        let old = self.active.replace(staged);
        self.applied_generation = generation;
        Ok(old)
    }

    pub fn disable_generation<E>(
        &mut self,
        generation: u64,
        drain: impl FnOnce(&mut T) -> Result<(), E>,
    ) -> Result<Option<T>, ApplyError<E>> {
        self.observe_desired(generation)
            .map_err(ApplyError::State)?;
        if generation < self.applied_generation {
            return Err(ApplyError::State(TransitionError::GenerationRollback));
        }
        if let Some(active) = self.active.as_mut() {
            drain(active).map_err(ApplyError::External)?;
        }
        let old = self.active.take();
        self.applied_generation = generation;
        Ok(old)
    }

    pub fn invalidate<E>(
        &mut self,
        drain: impl FnOnce(&mut T) -> Result<(), E>,
    ) -> Result<Option<T>, ApplyError<E>> {
        if let Some(active) = self.active.as_mut() {
            drain(active).map_err(ApplyError::External)?;
        }
        let old = self.active.take();
        self.applied_generation = 0;
        Ok(old)
    }

    /// Restores the still-held prior listener when publishing applied state fails after an
    /// in-memory switchover. The newly staged listener is returned for prompt drain/drop.
    pub fn rollback_switchover(&mut self, old: Option<T>, prior_generation: u64) -> Option<T> {
        let failed = self.active.take();
        self.active = old;
        self.applied_generation = prior_generation;
        failed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TransitionError {
    #[error("listener generation rollback is forbidden")]
    GenerationRollback,
    #[error("listener generation is already applied")]
    GenerationNotNew,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ApplyError<E> {
    #[error(transparent)]
    State(TransitionError),
    #[error("listener apply phase failed")]
    External(E),
}

#[cfg(test)]
mod tests {
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
}
