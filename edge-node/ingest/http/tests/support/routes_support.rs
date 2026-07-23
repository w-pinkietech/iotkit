use super::*;

impl HttpIngestConfig {
    pub(crate) fn for_test() -> Self {
        Self {
            admission: AdmissionConfig::for_test(),
            ..Self::default()
        }
    }
}

impl HttpIngestHooks {
    pub(crate) fn with_before_cached_reserved_admission(
        mut self,
        hook: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        self.before_cached_reserved_admission = Some(Arc::new(hook));
        self
    }

    pub(crate) fn with_after_cached_reserved_admission(
        mut self,
        hook: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        self.after_cached_reserved_admission = Some(Arc::new(hook));
        self
    }

    pub(crate) fn with_before_collector_handoff(
        mut self,
        hook: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        self.before_collector_handoff = Some(Arc::new(hook));
        self
    }

    pub(crate) fn with_after_queue_acquired(
        mut self,
        hook: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        self.after_queue_acquired = Some(Arc::new(hook));
        self
    }

    pub(crate) fn with_after_collector_result(
        mut self,
        hook: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        self.after_collector_result = Some(Arc::new(hook));
        self
    }

    pub(crate) fn with_after_response_serialization(
        mut self,
        hook: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        self.after_response_serialization = Some(Arc::new(hook));
        self
    }
}

impl<C: MonotonicClock> HttpIngestService<C> {
    pub(crate) fn new_with_hooks(
        db: DbHandle,
        collector: Collector,
        issuer: DevicePrincipalIssuer,
        config: HttpIngestConfig,
        clock: C,
        hooks: HttpIngestHooks,
    ) -> Result<Self, InvalidHttpIngestConfig> {
        Self::new_inner(db, collector, issuer, config, clock, hooks)
    }

    pub(crate) fn admission_snapshot(&self) -> crate::admission::test_support::AdmissionSnapshot {
        self.shared.admission.snapshot()
    }

    pub(crate) fn auth_cache_contains(&self, bearer: &str) -> bool {
        let key: [u8; 32] = Sha256::digest(bearer.as_bytes()).into();
        self.shared
            .cache
            .lock()
            .expect("auth cache mutex poisoned")
            .entries
            .contains_key(&key)
    }
}
