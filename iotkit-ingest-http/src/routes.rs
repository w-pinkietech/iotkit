use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, Response, StatusCode, header};
use http_body_util::BodyExt;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use iotkit_core_collector::{
    AuthenticatedDeviceIdentity, Collector, DeviceAuthorityProof, DevicePrincipalIssuer,
    IngestRequest, SubmitError,
};
use iotkit_core_ops::DeviceAuthentication;
use iotkit_core_storage::DbHandle;
use iotkit_ingest_contract::{
    AckStatus, Envelope, EnvelopeAck, ReasonCode, ValidationIssue, ValidationReport,
};
use sha2::{Digest, Sha256};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::{AdmissionConfig, AdmissionController, AdmissionDenied, MonotonicClock};

const ITEM_WORK_UNITS: u64 = 256;

#[derive(Debug, Clone)]
pub struct HttpIngestConfig {
    pub admission: AdmissionConfig,
    pub max_header_count: usize,
    pub max_header_bytes: usize,
    pub max_body_bytes: usize,
    pub max_items: usize,
    pub concurrent_requests: usize,
    pub concurrent_connections: usize,
    pub collector_queue_slots: usize,
    pub auth_cache_capacity: usize,
    pub auth_cache_ttl_ms: u64,
    pub read_timeout: Duration,
    pub whole_request_timeout: Duration,
    pub collector_timeout: Duration,
    pub retry_after_seconds: u64,
}

impl HttpIngestConfig {
    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        Self {
            admission: AdmissionConfig::for_test(),
            ..Self::default()
        }
    }

    fn validate(&self) -> Result<(), InvalidHttpIngestConfig> {
        if self.max_header_count == 0
            || self.max_header_bytes == 0
            || self.max_body_bytes == 0
            || self.max_items == 0
            || self.max_items > iotkit_core_collector::MAX_ITEMS_PER_ENVELOPE
            || self.concurrent_requests == 0
            || self.concurrent_connections == 0
            || self.collector_queue_slots == 0
            || self.auth_cache_capacity == 0
            || self.auth_cache_ttl_ms == 0
            || self.read_timeout.is_zero()
            || self.whole_request_timeout.is_zero()
            || self.collector_timeout.is_zero()
            || !(1..=3600).contains(&self.retry_after_seconds)
        {
            return Err(InvalidHttpIngestConfig);
        }
        Ok(())
    }
}

impl Default for HttpIngestConfig {
    fn default() -> Self {
        Self {
            admission: AdmissionConfig::default(),
            max_header_count: 32,
            max_header_bytes: 8 * 1024,
            max_body_bytes: 64 * 1024,
            max_items: iotkit_core_collector::MAX_ITEMS_PER_ENVELOPE,
            concurrent_requests: 16,
            concurrent_connections: 32,
            collector_queue_slots: 8,
            auth_cache_capacity: 64,
            auth_cache_ttl_ms: 60_000,
            read_timeout: Duration::from_secs(5),
            whole_request_timeout: Duration::from_secs(10),
            collector_timeout: Duration::from_secs(5),
            retry_after_seconds: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidHttpIngestConfig;

impl std::fmt::Display for InvalidHttpIngestConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HTTP ingest limits must be finite, positive, and internally consistent")
    }
}

impl std::error::Error for InvalidHttpIngestConfig {}

struct CachedAuthentication {
    authentication: DeviceAuthentication,
    expires_at_ms: u64,
}

struct AuthCache {
    entries: HashMap<[u8; 32], CachedAuthentication>,
    order: VecDeque<[u8; 32]>,
}

struct Shared<C: MonotonicClock> {
    db: DbHandle,
    collector: Collector,
    issuer: Mutex<DevicePrincipalIssuer>,
    config: HttpIngestConfig,
    admission: AdmissionController<C>,
    requests: Arc<Semaphore>,
    connections: Arc<Semaphore>,
    queue: Arc<Semaphore>,
    cache: Mutex<AuthCache>,
    clock: C,
    hooks: HttpIngestHooks,
}

#[derive(Clone, Default)]
pub(crate) struct HttpIngestHooks {
    before_cached_reserved_admission: Option<Arc<dyn Fn() + Send + Sync>>,
    after_cached_reserved_admission: Option<Arc<dyn Fn() + Send + Sync>>,
    before_collector_handoff: Option<Arc<dyn Fn() + Send + Sync>>,
    after_queue_acquired: Option<Arc<dyn Fn() + Send + Sync>>,
    after_collector_result: Option<Arc<dyn Fn() + Send + Sync>>,
    after_response_serialization: Option<Arc<dyn Fn() + Send + Sync>>,
}

#[cfg(test)]
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

pub struct HttpIngestService<C: MonotonicClock> {
    shared: Arc<Shared<C>>,
}

impl<C: MonotonicClock> Clone for HttpIngestService<C> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<C: MonotonicClock> HttpIngestService<C> {
    pub fn new(
        db: DbHandle,
        collector: Collector,
        issuer: DevicePrincipalIssuer,
        config: HttpIngestConfig,
        clock: C,
    ) -> Result<Self, InvalidHttpIngestConfig> {
        Self::new_inner(
            db,
            collector,
            issuer,
            config,
            clock,
            HttpIngestHooks::default(),
        )
    }

    #[cfg(test)]
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

    fn new_inner(
        db: DbHandle,
        collector: Collector,
        issuer: DevicePrincipalIssuer,
        config: HttpIngestConfig,
        clock: C,
        hooks: HttpIngestHooks,
    ) -> Result<Self, InvalidHttpIngestConfig> {
        config.validate()?;
        let admission = AdmissionController::new(config.admission.clone(), clock.clone())
            .map_err(|_| InvalidHttpIngestConfig)?;
        Ok(Self {
            shared: Arc::new(Shared {
                db,
                collector,
                issuer: Mutex::new(issuer),
                requests: Arc::new(Semaphore::new(config.concurrent_requests)),
                connections: Arc::new(Semaphore::new(config.concurrent_connections)),
                queue: Arc::new(Semaphore::new(config.collector_queue_slots)),
                cache: Mutex::new(AuthCache {
                    entries: HashMap::with_capacity(config.auth_cache_capacity),
                    order: VecDeque::with_capacity(config.auth_cache_capacity),
                }),
                config,
                admission,
                clock,
                hooks,
            }),
        })
    }

    #[cfg(test)]
    pub(crate) fn admission_snapshot(&self) -> crate::admission::AdmissionSnapshot {
        self.shared.admission.snapshot()
    }

    pub fn admission_health(&self) -> crate::AdmissionHealthSnapshot {
        self.shared.admission.note_current_capacity_pressure(
            self.shared
                .config
                .collector_queue_slots
                .saturating_sub(self.shared.queue.available_permits()),
            self.shared.config.collector_queue_slots,
            self.shared
                .config
                .concurrent_requests
                .saturating_sub(self.shared.requests.available_permits()),
            self.shared.config.concurrent_requests,
            self.shared
                .config
                .concurrent_connections
                .saturating_sub(self.shared.connections.available_permits()),
            self.shared.config.concurrent_connections,
        );
        self.shared.admission.health_snapshot()
    }

    pub fn pending_throttle_episode_events(&self) -> Vec<crate::ThrottleEpisodeEvent> {
        self.shared.admission.pending_episode_events()
    }

    pub fn acknowledge_throttle_episode_events(
        &self,
        events: &[crate::ThrottleEpisodeEvent],
    ) -> bool {
        self.shared.admission.acknowledge_episode_events(events)
    }

    #[cfg(test)]
    pub(crate) fn auth_cache_contains(&self, bearer: &str) -> bool {
        let key: [u8; 32] = Sha256::digest(bearer.as_bytes()).into();
        self.shared
            .cache
            .lock()
            .expect("auth cache mutex poisoned")
            .entries
            .contains_key(&key)
    }

    pub async fn handle(&self, observed_peer: IpAddr, request: Request<Body>) -> Response<Body> {
        let request_permit = match self.shared.requests.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                self.shared.admission.record_throttled_drop();
                return self.retry_response(StatusCode::TOO_MANY_REQUESTS);
            }
        };
        match tokio::time::timeout(
            self.shared.config.whole_request_timeout,
            self.handle_bounded(observed_peer, request, request_permit),
        )
        .await
        {
            Ok(response) => response,
            Err(_) => response(StatusCode::REQUEST_TIMEOUT, Body::empty()),
        }
    }

    /// Serve a previously site-CIDR-validated stream with hard header and connection limits.
    /// The listener readiness gate remains owned by the gateway composition boundary.
    pub async fn serve_connection(
        &self,
        stream: crate::AcceptedStream,
        observed_peer: std::net::SocketAddr,
    ) -> Result<(), ServeConnectionError> {
        let _connection = match self.shared.connections.clone().try_acquire_owned() {
            Ok(connection) => connection,
            Err(_) => {
                self.shared.admission.record_throttled_drop();
                return Err(ServeConnectionError::Busy);
            }
        };
        let service = self.clone();
        let hyper_service = service_fn(move |request: Request<hyper::body::Incoming>| {
            let service = service.clone();
            async move {
                Ok::<_, std::convert::Infallible>(
                    service
                        .handle(observed_peer.ip(), request.map(Body::new))
                        .await,
                )
            }
        });
        let mut builder = http1::Builder::new();
        builder
            .max_headers(self.shared.config.max_header_count)
            .max_buf_size(self.shared.config.max_header_bytes.max(8192))
            // Hyper restarts this timer whenever it begins reading a request head. It therefore
            // bounds both the initial/partial header and the next header on an idle keep-alive
            // connection without shortening `whole_request_timeout` after parsing completes.
            .header_read_timeout(self.shared.config.read_timeout)
            .timer(TokioTimer::new());
        let result = builder
            .serve_connection(TokioIo::new(stream), hyper_service)
            .await;
        match result {
            Ok(()) => Ok(()),
            Err(error) if error.is_timeout() => Err(ServeConnectionError::HeaderReadTimeout),
            Err(_) => Err(ServeConnectionError::Protocol),
        }
    }

    async fn handle_bounded(
        &self,
        observed_peer: IpAddr,
        request: Request<Body>,
        _request_permit: OwnedSemaphorePermit,
    ) -> Response<Body> {
        if request.method() != Method::POST {
            return response(StatusCode::METHOD_NOT_ALLOWED, Body::empty());
        }
        let validate = match request.uri().path() {
            "/api/v1/ingest" => false,
            "/api/v1/ingest/validate" => true,
            _ => return response(StatusCode::NOT_FOUND, Body::empty()),
        };
        if header_cost(request.headers()) > self.shared.config.max_header_bytes
            || request.headers().len() > self.shared.config.max_header_count
        {
            return response(StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE, Body::empty());
        }
        if request.headers().contains_key(header::CONTENT_ENCODING) {
            return response(StatusCode::UNSUPPORTED_MEDIA_TYPE, Body::empty());
        }
        if request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_none_or(|value| value.split(';').next() != Some("application/json"))
        {
            return response(StatusCode::UNSUPPORTED_MEDIA_TYPE, Body::empty());
        }
        let declared_length = match content_length(request.headers()) {
            Ok(Some(length)) if length <= self.shared.config.max_body_bytes => Some(length),
            Ok(Some(_)) => return response(StatusCode::PAYLOAD_TOO_LARGE, Body::empty()),
            Ok(None) => None,
            Err(()) => return response(StatusCode::BAD_REQUEST, Body::empty()),
        };
        let Some(bearer) = bearer(request.headers()) else {
            return response(StatusCode::UNAUTHORIZED, Body::empty());
        };
        let token_key: [u8; 32] = Sha256::digest(bearer.as_bytes()).into();
        let auth_permit = if let Some(authentication) = self.cache_get(&token_key) {
            if let Some(hook) = &self.shared.hooks.before_cached_reserved_admission {
                hook();
                tokio::task::yield_now().await;
            }
            let shared = Arc::clone(&self.shared);
            let cached_admission = self
                .shared
                .db
                .with_conn(move |conn| {
                    let tx = rusqlite::Transaction::new_unchecked(
                        conn,
                        rusqlite::TransactionBehavior::Immediate,
                    )?;
                    let current = iotkit_core_ops::authentication_is_current(&tx, &authentication);
                    let admission = match current {
                        Ok(true) => Some(shared.admission.try_begin_auth(observed_peer, true)),
                        Ok(false) => None,
                        Err(error) => return Ok(Err(error)),
                    };
                    if admission.as_ref().is_some_and(Result::is_ok)
                        && let Some(hook) = &shared.hooks.after_cached_reserved_admission
                    {
                        hook();
                    }
                    tx.commit()?;
                    Ok(Ok(admission))
                })
                .await;
            match cached_admission {
                Ok(Ok(Some(Ok(permit)))) => permit,
                Ok(Ok(Some(Err(_)))) => {
                    return self.retry_response(StatusCode::TOO_MANY_REQUESTS);
                }
                Ok(Ok(None)) => {
                    self.cache_remove(&token_key);
                    match self.shared.admission.try_begin_auth(observed_peer, false) {
                        Ok(permit) => permit,
                        Err(_) => return self.retry_response(StatusCode::TOO_MANY_REQUESTS),
                    }
                }
                Ok(Err(_)) | Err(_) => {
                    return response(StatusCode::SERVICE_UNAVAILABLE, Body::empty());
                }
            }
        } else {
            match self.shared.admission.try_begin_auth(observed_peer, false) {
                Ok(permit) => permit,
                Err(_) => return self.retry_response(StatusCode::TOO_MANY_REQUESTS),
            }
        };

        let authentication = match self.authenticate(&bearer, &token_key, validate).await {
            Ok(Some(authentication)) => authentication,
            Ok(None) => {
                self.shared.admission.record_auth_failure(observed_peer);
                return response(StatusCode::UNAUTHORIZED, Body::empty());
            }
            Err(()) => return response(StatusCode::SERVICE_UNAVAILABLE, Body::empty()),
        };
        if !self.recheck(&authentication).await {
            self.cache_remove(&token_key);
            return response(StatusCode::UNAUTHORIZED, Body::empty());
        }

        let principal_record = authentication.principal();
        let principal = self
            .shared
            .issuer
            .lock()
            .expect("device principal issuer mutex poisoned")
            .authenticated_device(
                AuthenticatedDeviceIdentity::new(
                    principal_record.principal_id(),
                    principal_record.credential_id(),
                    principal_record.principal_id(),
                    principal_record.flow_class(),
                ),
                principal_record.scopes().iter().copied(),
                DeviceAuthorityProof::new(
                    principal_record.auth_epoch(),
                    authentication.auth_generation(),
                    authentication.principal_material_generation(),
                ),
            );
        let pre_body_bytes = declared_length.unwrap_or(self.shared.config.max_body_bytes);
        let maximum_cost = 1_u64
            .saturating_add(pre_body_bytes as u64)
            .saturating_add((self.shared.config.max_items as u64).saturating_mul(ITEM_WORK_UNITS));
        let mut reservation = match self.shared.admission.reserve_principal(
            principal.principal_id(),
            principal_record.flow_class(),
            maximum_cost,
        ) {
            Ok(reservation) => reservation,
            Err(_) => return self.retry_response(StatusCode::TOO_MANY_REQUESTS),
        };
        drop(auth_permit);

        let body = match read_bounded(
            request.into_body(),
            self.shared.config.max_body_bytes,
            self.shared.config.read_timeout,
            &mut reservation,
        )
        .await
        {
            Ok(body) => body,
            Err(ReadError::TooLarge) => {
                return response(StatusCode::PAYLOAD_TOO_LARGE, Body::empty());
            }
            Err(ReadError::Timeout) => return response(StatusCode::REQUEST_TIMEOUT, Body::empty()),
            Err(ReadError::Transport) => return response(StatusCode::BAD_REQUEST, Body::empty()),
        };
        let envelope: Envelope = match serde_json::from_slice(&body) {
            Ok(envelope) => envelope,
            Err(_) => return response(StatusCode::BAD_REQUEST, Body::empty()),
        };
        if envelope.items.len() > self.shared.config.max_items {
            if reservation
                .reconcile_at(body.len(), envelope.items.len(), self.shared.clock.now_ms())
                .is_err()
            {
                return self.retry_response(StatusCode::TOO_MANY_REQUESTS);
            }
            return if validate {
                json(
                    StatusCode::OK,
                    &ValidationReport {
                        envelope_id: envelope.envelope_id,
                        valid: false,
                        issues: vec![ValidationIssue {
                            item_index: None,
                            reason_code: ReasonCode::BatchTooLarge,
                            message: format!(
                                "items {} > {}",
                                envelope.items.len(),
                                self.shared.config.max_items
                            ),
                            field_path: Some("/items".into()),
                            schema_hint: Some(format!(
                                "at most {} items",
                                self.shared.config.max_items
                            )),
                        }],
                    },
                )
            } else {
                json(
                    StatusCode::OK,
                    &EnvelopeAck {
                        envelope_id: envelope.envelope_id,
                        status: AckStatus::Rejected {
                            reason_code: ReasonCode::BatchTooLarge,
                            message: format!(
                                "items {} > {}",
                                envelope.items.len(),
                                self.shared.config.max_items
                            ),
                            field_path: Some("/items".into()),
                            schema_hint: Some(format!(
                                "at most {} items",
                                self.shared.config.max_items
                            )),
                        },
                    },
                )
            };
        }
        if reservation
            .reconcile_at(body.len(), envelope.items.len(), self.shared.clock.now_ms())
            .is_err()
        {
            return self.retry_response(StatusCode::TOO_MANY_REQUESTS);
        }
        if !self.recheck(&authentication).await {
            self.cache_remove(&token_key);
            return response(StatusCode::UNAUTHORIZED, Body::empty());
        }
        if let Some(hook) = &self.shared.hooks.before_collector_handoff {
            hook();
            tokio::task::yield_now().await;
        }
        let _queue_permit = match self.shared.queue.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                self.shared.admission.record_throttled_drop();
                return response(StatusCode::SERVICE_UNAVAILABLE, Body::empty());
            }
        };
        self.shared.admission.note_queue_depth(
            self.shared
                .config
                .collector_queue_slots
                .saturating_sub(self.shared.queue.available_permits()),
        );
        if let Some(hook) = &self.shared.hooks.after_queue_acquired {
            hook();
            tokio::task::yield_now().await;
        }
        let ingest = IngestRequest {
            principal,
            envelope,
        };
        if validate {
            match tokio::time::timeout(
                self.shared.config.collector_timeout,
                self.shared.collector.validate(ingest),
            )
            .await
            {
                Ok(Ok(report)) => self.collector_success_response(&report).await,
                Ok(Err(SubmitError::AuthenticationStale)) => {
                    response(StatusCode::UNAUTHORIZED, Body::empty())
                }
                Ok(Err(SubmitError::ClockUntrusted | SubmitError::NoAck | SubmitError::Closed))
                | Err(_) => response(StatusCode::SERVICE_UNAVAILABLE, Body::empty()),
            }
        } else {
            match tokio::time::timeout(
                self.shared.config.collector_timeout,
                self.shared.collector.submit(ingest),
            )
            .await
            {
                Ok(Ok(ack)) => self.collector_success_response(&ack).await,
                Ok(Err(SubmitError::AuthenticationStale)) => {
                    response(StatusCode::UNAUTHORIZED, Body::empty())
                }
                Ok(Err(SubmitError::ClockUntrusted | SubmitError::NoAck | SubmitError::Closed))
                | Err(_) => response(StatusCode::SERVICE_UNAVAILABLE, Body::empty()),
            }
        }
    }

    async fn collector_success_response<T: serde::Serialize>(&self, value: &T) -> Response<Body> {
        if let Some(hook) = &self.shared.hooks.after_collector_result {
            hook();
            tokio::task::yield_now().await;
        }
        let response = json(StatusCode::OK, value);
        if let Some(hook) = &self.shared.hooks.after_response_serialization {
            hook();
            tokio::task::yield_now().await;
        }
        response
    }

    async fn authenticate(
        &self,
        bearer: &str,
        key: &[u8; 32],
        read_only: bool,
    ) -> Result<Option<DeviceAuthentication>, ()> {
        if let Some(authentication) = self.cache_get(key) {
            return Ok(Some(authentication));
        }
        let secret = bearer.to_owned();
        let result = self
            .shared
            .db
            .with_conn(move |conn| {
                Ok(if read_only {
                    iotkit_core_ops::inspect_device_credential(conn, &secret)
                } else {
                    iotkit_core_ops::authenticate_device(conn, &secret)
                })
            })
            .await
            .map_err(|_| ())?
            .map_err(|_| ())?;
        if !read_only && let Some(authentication) = &result {
            self.cache_insert(*key, authentication.clone());
        }
        Ok(result)
    }

    async fn recheck(&self, authentication: &DeviceAuthentication) -> bool {
        let authentication = authentication.clone();
        self.shared
            .db
            .with_conn(move |conn| {
                Ok(iotkit_core_ops::authentication_is_current(
                    conn,
                    &authentication,
                ))
            })
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(false)
    }

    fn cache_get(&self, key: &[u8; 32]) -> Option<DeviceAuthentication> {
        let now = self.shared.clock.now_ms();
        let mut cache = self.shared.cache.lock().expect("auth cache mutex poisoned");
        if cache
            .entries
            .get(key)
            .is_some_and(|entry| entry.expires_at_ms <= now)
        {
            cache.entries.remove(key);
            cache.order.retain(|candidate| candidate != key);
            return None;
        }
        cache
            .entries
            .get(key)
            .map(|entry| entry.authentication.clone())
    }

    fn cache_insert(&self, key: [u8; 32], authentication: DeviceAuthentication) {
        let mut cache = self.shared.cache.lock().expect("auth cache mutex poisoned");
        if !cache.entries.contains_key(&key) {
            while cache.entries.len() >= self.shared.config.auth_cache_capacity {
                if let Some(oldest) = cache.order.pop_front() {
                    cache.entries.remove(&oldest);
                } else {
                    break;
                }
            }
            cache.order.push_back(key);
        }
        cache.entries.insert(
            key,
            CachedAuthentication {
                authentication,
                expires_at_ms: self
                    .shared
                    .clock
                    .now_ms()
                    .saturating_add(self.shared.config.auth_cache_ttl_ms),
            },
        );
    }

    fn cache_remove(&self, key: &[u8; 32]) {
        let mut cache = self.shared.cache.lock().expect("auth cache mutex poisoned");
        cache.entries.remove(key);
        cache.order.retain(|candidate| candidate != key);
    }

    fn retry_response(&self, status: StatusCode) -> Response<Body> {
        let mut response = response(status, Body::empty());
        response.headers_mut().insert(
            header::RETRY_AFTER,
            self.shared
                .config
                .retry_after_seconds
                .to_string()
                .parse()
                .expect("validated Retry-After"),
        );
        response
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ServeConnectionError {
    #[error("HTTP ingress connection capacity is full")]
    Busy,
    #[error("HTTP ingress request header read timed out")]
    HeaderReadTimeout,
    #[error("HTTP ingress protocol error")]
    Protocol,
}

fn header_cost(headers: &axum::http::HeaderMap) -> usize {
    headers.iter().fold(0_usize, |cost, (name, value)| {
        cost.saturating_add(name.as_str().len())
            .saturating_add(value.as_bytes().len())
            .saturating_add(4)
    })
}

fn content_length(headers: &axum::http::HeaderMap) -> Result<Option<usize>, ()> {
    headers
        .get(header::CONTENT_LENGTH)
        .map(|value| value.to_str().map_err(|_| ())?.parse().map_err(|_| ()))
        .transpose()
}

fn bearer(headers: &axum::http::HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let secret = value.strip_prefix("Bearer ")?;
    (!secret.is_empty()).then(|| secret.to_owned())
}

enum ReadError {
    TooLarge,
    Timeout,
    Transport,
}

async fn read_bounded(
    mut body: Body,
    maximum: usize,
    timeout: Duration,
    reservation: &mut crate::PrincipalReservation,
) -> Result<Vec<u8>, ReadError> {
    let read = async {
        let mut output = Vec::with_capacity(maximum.min(4096));
        while let Some(frame) = body.frame().await {
            let frame = frame.map_err(|_| ReadError::Transport)?;
            if let Ok(data) = frame.into_data() {
                reservation.note_consumed_bytes(data.len());
                if output.len().saturating_add(data.len()) > maximum {
                    return Err(ReadError::TooLarge);
                }
                output.extend_from_slice(&data);
            }
        }
        Ok(output)
    };
    tokio::time::timeout(timeout, read)
        .await
        .map_err(|_| ReadError::Timeout)?
}

fn json<T: serde::Serialize>(status: StatusCode, value: &T) -> Response<Body> {
    match serde_json::to_vec(value) {
        Ok(body) => {
            let mut response = response(status, Body::from(body));
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static("application/json"),
            );
            response
        }
        Err(_) => response(StatusCode::SERVICE_UNAVAILABLE, Body::empty()),
    }
}

fn response(status: StatusCode, body: Body) -> Response<Body> {
    Response::builder()
        .status(status)
        .body(body)
        .expect("static HTTP response")
}

impl From<AdmissionDenied> for StatusCode {
    fn from(_: AdmissionDenied) -> Self {
        StatusCode::TOO_MANY_REQUESTS
    }
}

#[cfg(test)]
#[path = "../tests/routes.rs"]
mod tests;
