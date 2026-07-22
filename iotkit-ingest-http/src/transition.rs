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
#[path = "../tests/unit/transition_tests.rs"]
mod tests;
