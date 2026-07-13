//! Safe transport-construction boundary for the HTTP ingest listener.
//!
//! HTTP routes and ingest-domain behavior deliberately do not live here yet. Task 4 only owns
//! exposure classification, TLS material validation, peer checks, and socket construction.
//!
//! Test-only clocks, configuration constructors, and state probes are deliberately unavailable
//! to default external consumers:
//!
//! A normal external consumer cannot provide an alternate admission clock:
//!
//! ```compile_fail
//! #[derive(Clone)]
//! struct ExternalManualClock(u64);
//!
//! impl iotkit_ingest_http::MonotonicClock for ExternalManualClock {
//!     fn now_ms(&self) -> u64 {
//!         self.0
//!     }
//! }
//!
//! let _ = iotkit_ingest_http::AdmissionController::new(
//!     iotkit_ingest_http::AdmissionConfig::default(),
//!     ExternalManualClock(7),
//! )?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! The fixed production clock path remains available to normal external consumers:
//!
//! ```
//! let _ = iotkit_ingest_http::AdmissionController::new(
//!     iotkit_ingest_http::AdmissionConfig::default(),
//!     iotkit_ingest_http::SystemMonotonicClock::default(),
//! )?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ```compile_fail
//! use iotkit_ingest_http::ManualMonotonicClock;
//! let _ = ManualMonotonicClock::new(0);
//! ```
//!
//! ```compile_fail
//! let _ = iotkit_ingest_http::AdmissionConfig::for_test();
//! ```
//!
//! ```compile_fail
//! let _ = iotkit_ingest_http::HttpIngestConfig::for_test();
//! ```
//!
//! ```compile_fail
//! # use std::net::{IpAddr, Ipv4Addr};
//! # let clock = iotkit_ingest_http::SystemMonotonicClock::default();
//! # let admission = iotkit_ingest_http::AdmissionController::new(Default::default(), clock)?;
//! let _ = admission.pre_auth_source_count();
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ```compile_fail
//! # use std::net::{IpAddr, Ipv4Addr};
//! let _ = iotkit_ingest_http::ExposureSnapshot::new(
//!     "eth0",
//!     [IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2))],
//!     false,
//! );
//! ```
//!
//! ```compile_fail
//! # let exposure = iotkit_ingest_http::ExposureSnapshot::from_os("eth0")?;
//! let _ = exposure.interface();
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ```compile_fail
//! # let exposure = iotkit_ingest_http::ExposureSnapshot::from_os("eth0")?;
//! let _ = exposure.addresses();
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ```compile_fail
//! # let exposure = iotkit_ingest_http::ExposureSnapshot::from_os("eth0")?;
//! let _ = exposure.internet_default_route();
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ```compile_fail
//! # let exposure = iotkit_ingest_http::ExposureSnapshot::from_os("eth0")?;
//! # let config: iotkit_ingest_http::ListenerConfig = unimplemented!();
//! let _ = iotkit_ingest_http::ValidatedListenerConfig::new_for_test(config, &exposure);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::collections::BTreeSet;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::task::{Context, Poll};

use ipnet::IpNet;
use rustls::ServerConfig;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_rustls::TlsAcceptor;

const TLS_HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[cfg(test)]
extern crate self as iotkit_ingest_http;

mod transition;
pub use transition::{ApplyError, ListenerTransition, TransitionError};

mod admission;
#[cfg(test)]
pub(crate) use admission::ManualMonotonicClock;
pub use admission::{
    AdmissionConfig, AdmissionController, AdmissionDenied, AdmissionHealthSnapshot, AuthPermit,
    FlowClassLimit, InvalidAdmissionConfig, MonotonicClock, PrincipalReservation,
    SystemMonotonicClock, ThrottleEpisodeEvent,
};

mod routes;
pub use routes::{
    HttpIngestConfig, HttpIngestService, InvalidHttpIngestConfig, ServeConnectionError,
};

pub type SiteCidr = IpNet;

#[derive(Clone)]
pub enum ListenerMode {
    Tls(TlsMaterial),
    PrivatePlaintext,
}

impl std::fmt::Debug for ListenerMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tls(material) => f
                .debug_struct("Tls")
                .field("generation", &material.generation)
                .field("fingerprint", &material.fingerprint)
                .field("private_key", &"[REDACTED]")
                .finish(),
            Self::PrivatePlaintext => f.write_str("PrivatePlaintext"),
        }
    }
}

#[derive(Clone)]
pub struct TlsMaterial {
    cert_pem: Arc<[u8]>,
    key_pem: Arc<[u8]>,
    fingerprint: String,
    generation: u64,
}

impl std::fmt::Debug for TlsMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsMaterial")
            .field("generation", &self.generation)
            .field("fingerprint", &self.fingerprint)
            .field("certificate", &"[PUBLIC CERTIFICATE]")
            .field("private_key", &"[REDACTED]")
            .finish()
    }
}

impl TlsMaterial {
    pub fn validate(
        cert_pem: Vec<u8>,
        key_pem: Vec<u8>,
        expected_fingerprint: &str,
        generation: u64,
    ) -> Result<Self, ListenerError> {
        if cert_pem.is_empty() || key_pem.is_empty() {
            return Err(ListenerError::IncompleteTls);
        }
        if cert_pem.len() > 1024 * 1024 || key_pem.len() > 1024 * 1024 {
            return Err(ListenerError::TlsMaterialTooLarge);
        }
        if generation == 0 {
            return Err(ListenerError::InvalidTlsGeneration);
        }
        let fingerprint = certificate_fingerprint(&cert_pem)?;
        if fingerprint != expected_fingerprint {
            return Err(ListenerError::TlsFingerprintMismatch);
        }
        build_server_config(&cert_pem, &key_pem)?;
        Ok(Self {
            cert_pem: cert_pem.into(),
            key_pem: key_pem.into(),
            fingerprint,
            generation,
        })
    }

    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn server_config(&self) -> Result<Arc<ServerConfig>, ListenerError> {
        build_server_config(&self.cert_pem, &self.key_pem).map(Arc::new)
    }
}

#[derive(Debug, Clone)]
pub struct ListenerConfig {
    pub bind: SocketAddr,
    pub interface: String,
    pub site_local_cidrs: Vec<SiteCidr>,
    pub mode: ListenerMode,
}

#[derive(Debug, Clone)]
pub struct ExposureSnapshot {
    interface: String,
    addresses: BTreeSet<IpAddr>,
    _internet_default_route: bool,
}

impl ExposureSnapshot {
    /// Reads interface addresses and default-route state from the running OS. The default product
    /// listener composition uses this trusted producer; alternate inventory snapshots must enter
    /// through the explicit composition boundary and do not change validation rules.
    pub fn from_os(interface: &str) -> Result<Self, ListenerError> {
        let mut addresses = BTreeSet::new();
        let mut head = std::ptr::null_mut();
        // SAFETY: getifaddrs initializes `head` on success; every pointer is checked before use
        // and the list is released exactly once below.
        if unsafe { libc::getifaddrs(&mut head) } != 0 {
            return Err(ListenerError::Io(io::Error::last_os_error()));
        }
        let mut current = head;
        while !current.is_null() {
            // SAFETY: `current` belongs to the live getifaddrs list.
            let entry = unsafe { &*current };
            if !entry.ifa_name.is_null() && !entry.ifa_addr.is_null() {
                // SAFETY: ifa_name is a NUL-terminated C string by contract.
                let name = unsafe { std::ffi::CStr::from_ptr(entry.ifa_name) };
                if name.to_bytes() == interface.as_bytes() {
                    // SAFETY: family selects the corresponding sockaddr layout.
                    let family = unsafe { (*entry.ifa_addr).sa_family as i32 };
                    if family == libc::AF_INET {
                        let address = unsafe { &*(entry.ifa_addr.cast::<libc::sockaddr_in>()) };
                        addresses.insert(IpAddr::V4(std::net::Ipv4Addr::from(u32::from_be(
                            address.sin_addr.s_addr,
                        ))));
                    } else if family == libc::AF_INET6 {
                        let address = unsafe { &*(entry.ifa_addr.cast::<libc::sockaddr_in6>()) };
                        addresses.insert(normalize_ip(IpAddr::V6(std::net::Ipv6Addr::from(
                            address.sin6_addr.s6_addr,
                        ))));
                    }
                }
            }
            current = entry.ifa_next;
        }
        // SAFETY: `head` is the list returned by the successful getifaddrs call above.
        unsafe { libc::freeifaddrs(head) };
        if addresses.is_empty() {
            return Err(ListenerError::UnapprovedInterface);
        }
        Ok(Self {
            interface: interface.to_owned(),
            addresses,
            _internet_default_route: interface_has_default_route(interface)?,
        })
    }

    /// Constructs a snapshot from an already-authoritative interface inventory.
    ///
    /// The default Edge composition uses [`Self::from_os`]. This boundary is for a trusted
    /// composition root that already owns the inventory source (for example, a supervisor that
    /// receives interface state from a platform service). It does not classify or authorize a
    /// bind by itself: [`ValidatedListenerConfig::new`] still requires a private address, a
    /// matching interface, and a site-local CIDR.
    pub fn from_inventory(
        interface: impl Into<String>,
        addresses: impl IntoIterator<Item = IpAddr>,
        internet_default_route: bool,
    ) -> Result<Self, ListenerError> {
        let interface = interface.into();
        if interface.trim().is_empty() || interface.len() > 64 {
            return Err(ListenerError::UnapprovedInterface);
        }
        let addresses = addresses
            .into_iter()
            .map(normalize_ip)
            .collect::<BTreeSet<_>>();
        if addresses.is_empty() {
            return Err(ListenerError::UnapprovedInterface);
        }
        Ok(Self {
            interface,
            addresses,
            _internet_default_route: internet_default_route,
        })
    }

    #[cfg(test)]
    pub(crate) fn new(
        interface: impl Into<String>,
        addresses: impl IntoIterator<Item = IpAddr>,
        internet_default_route: bool,
    ) -> Self {
        Self {
            interface: interface.into(),
            addresses: addresses.into_iter().map(normalize_ip).collect(),
            _internet_default_route: internet_default_route,
        }
    }

    #[cfg(test)]
    pub(crate) fn interface(&self) -> &str {
        &self.interface
    }

    #[cfg(test)]
    pub(crate) fn addresses(&self) -> &BTreeSet<IpAddr> {
        &self.addresses
    }
}

#[derive(Debug, Clone)]
pub struct ValidatedListenerConfig {
    config: ListenerConfig,
    degraded: bool,
}

impl ValidatedListenerConfig {
    pub fn new(config: ListenerConfig, exposure: &ExposureSnapshot) -> Result<Self, ListenerError> {
        Self::validate(config, exposure, false)
    }

    /// Loopback is deliberately available only to crate/integration tests. Product configuration
    /// must name a real private site interface and CIDR.
    #[cfg(test)]
    pub(crate) fn new_for_test(
        config: ListenerConfig,
        exposure: &ExposureSnapshot,
    ) -> Result<Self, ListenerError> {
        Self::validate(config, exposure, true)
    }

    fn validate(
        mut config: ListenerConfig,
        exposure: &ExposureSnapshot,
        allow_loopback: bool,
    ) -> Result<Self, ListenerError> {
        config.bind.set_ip(normalize_ip(config.bind.ip()));
        if config.interface.trim().is_empty()
            || config.interface.len() > 64
            || config.interface != exposure.interface
        {
            return Err(ListenerError::UnapprovedInterface);
        }
        if config.bind.ip().is_unspecified() {
            return Err(ListenerError::InternetCapableExposure);
        }
        if !exposure.addresses.contains(&config.bind.ip()) {
            return Err(ListenerError::BindNotOnInterface);
        }
        if config.site_local_cidrs.is_empty() || config.site_local_cidrs.len() > 8 {
            return Err(ListenerError::MissingSiteCidr);
        }
        for cidr in &config.site_local_cidrs {
            validate_cidr(cidr, allow_loopback)?;
        }
        if !config
            .site_local_cidrs
            .iter()
            .any(|cidr| cidr_contains(cidr, config.bind.ip()))
        {
            return Err(ListenerError::BindOutsideSiteCidr);
        }
        validate_private_ip(config.bind.ip(), allow_loopback)?;
        let degraded = matches!(config.mode, ListenerMode::PrivatePlaintext);
        Ok(Self { config, degraded })
    }

    pub fn is_degraded(&self) -> bool {
        self.degraded
    }

    pub fn warning(&self) -> Option<&'static str> {
        self.degraded.then_some("private_plaintext")
    }

    pub fn bind_addr(&self) -> SocketAddr {
        self.config.bind
    }

    pub fn mode(&self) -> &ListenerMode {
        &self.config.mode
    }

    pub fn site_local_cidrs(&self) -> &[SiteCidr] {
        &self.config.site_local_cidrs
    }
}

pub struct Listener {
    inner: tokio::net::TcpListener,
    policy: ListenerPolicy,
    configured_addr: SocketAddr,
}

/// Runtime transport policy staged independently from a bound socket. The Edge uses this
/// boundary to validate TLS/configuration changes before pausing and swapping a live listener.
#[derive(Clone)]
pub struct ListenerPolicy {
    site_local_cidrs: Vec<SiteCidr>,
    tls: Option<TlsAcceptor>,
}

pub struct ServingListener {
    configured_addr: SocketAddr,
    local_addr: SocketAddr,
    shutdown: watch::Sender<bool>,
    control: mpsc::Sender<ListenerCommand>,
    task: tokio::task::JoinHandle<()>,
}

enum ListenerCommand {
    Pause {
        response: oneshot::Sender<()>,
    },
    ReplacePolicy {
        policy: ListenerPolicy,
        response: oneshot::Sender<Result<ListenerPolicy, io::Error>>,
    },
    Resume {
        response: oneshot::Sender<()>,
    },
}

impl ServingListener {
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Returns the durable configuration endpoint whose generation this listener serves.
    ///
    /// This is intentionally separate from [`Self::local_addr`]: a `:0` configuration remains
    /// the desired/applied truth while the kernel-selected runtime port is reported separately.
    pub fn configured_addr(&self) -> SocketAddr {
        self.configured_addr
    }

    /// Stop accepting new peers and acknowledge only after the accept loop has entered the
    /// paused state. Existing connection tasks are allowed to drain.
    pub async fn pause(&self) -> io::Result<()> {
        let (response, result) = oneshot::channel();
        self.control
            .send(ListenerCommand::Pause { response })
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "listener task exited"))?;
        result
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "listener task exited"))?;
        Ok(())
    }

    /// Replace the runtime TLS/peer policy while the listener is paused and return the previous
    /// policy for rollback if durable applied-state publication fails.
    pub async fn replace_policy(&self, policy: ListenerPolicy) -> io::Result<ListenerPolicy> {
        let (response, result) = oneshot::channel();
        self.control
            .send(ListenerCommand::ReplacePolicy { policy, response })
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "listener task exited"))?;
        result
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "listener task exited"))?
    }

    /// Resume accepting peers after the durable generation boundary has completed.
    pub async fn resume(&self) -> io::Result<()> {
        let (response, result) = oneshot::channel();
        self.control
            .send(ListenerCommand::Resume { response })
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "listener task exited"))?;
        result
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "listener task exited"))?;
        Ok(())
    }

    /// Signal listener shutdown and wait until every supervised peer task has been
    /// drained or cancelled. This is the composition boundary used by orderly
    /// teardown; dropping still provides a best-effort signal for invalidation paths.
    pub async fn shutdown(mut self) {
        let _ = self.shutdown.send(true);
        let _ = (&mut self.task).await;
    }
}

impl Drop for ServingListener {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

impl Listener {
    pub fn stage_policy(config: &ValidatedListenerConfig) -> Result<ListenerPolicy, ListenerError> {
        Self::stage_policy_with_cidrs(config, config.site_local_cidrs().to_vec())
    }

    /// Stages policy for a socket supplied by the composition root.
    ///
    /// A loopback socket is explicitly treated as local-only and therefore accepts only
    /// loopback peers. This keeps the socket-injection boundary useful for deterministic
    /// supervisor composition without turning loopback into an accepted site-LAN exposure.
    pub fn stage_policy_for_local_addr(
        config: &ValidatedListenerConfig,
        local_addr: SocketAddr,
    ) -> Result<ListenerPolicy, ListenerError> {
        let local_ip = normalize_ip(local_addr.ip());
        let configured_ip = normalize_ip(config.bind_addr().ip());
        let peer_cidrs = if local_ip.is_loopback() {
            vec![loopback_cidr(local_ip)]
        } else if local_ip == configured_ip {
            config.site_local_cidrs().to_vec()
        } else {
            return Err(ListenerError::BindNotOnInterface);
        };
        Self::stage_policy_with_cidrs(config, peer_cidrs)
    }

    fn stage_policy_with_cidrs(
        config: &ValidatedListenerConfig,
        site_local_cidrs: Vec<SiteCidr>,
    ) -> Result<ListenerPolicy, ListenerError> {
        let tls = match config.mode() {
            ListenerMode::Tls(material) => Some(TlsAcceptor::from(material.server_config()?)),
            ListenerMode::PrivatePlaintext => None,
        };
        Ok(ListenerPolicy {
            site_local_cidrs,
            tls,
        })
    }

    pub async fn bind(config: ValidatedListenerConfig) -> Result<Self, ListenerError> {
        let policy = Self::stage_policy(&config)?;
        let bind_addr = config.bind_addr();
        let socket = match bind_addr.ip() {
            IpAddr::V4(_) => tokio::net::TcpSocket::new_v4()?,
            IpAddr::V6(_) => tokio::net::TcpSocket::new_v6()?,
        };
        #[cfg(not(windows))]
        socket.set_reuseaddr(true)?;
        socket.bind(bind_addr)?;
        let inner = socket.listen(128)?;
        Ok(Self {
            inner,
            policy,
            configured_addr: bind_addr,
        })
    }

    /// Adopts a socket owned by the composition root after the configuration has been strictly
    /// validated. This supports socket activation and deterministic supervisor probes without
    /// changing the production [`Self::bind`] path or its private-site checks.
    pub fn from_prebound_socket(
        config: ValidatedListenerConfig,
        inner: tokio::net::TcpListener,
    ) -> Result<Self, ListenerError> {
        let local_addr = inner.local_addr()?;
        let policy = Self::stage_policy_for_local_addr(&config, local_addr)?;
        Ok(Self {
            inner,
            policy,
            configured_addr: config.bind_addr(),
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }

    pub async fn accept(&self) -> Result<(AcceptedStream, SocketAddr), ListenerError> {
        let (stream, peer) = self.inner.accept().await?;
        Self::accept_stream(stream, peer, &self.policy).await
    }

    async fn accept_stream(
        stream: tokio::net::TcpStream,
        peer: SocketAddr,
        policy: &ListenerPolicy,
    ) -> Result<(AcceptedStream, SocketAddr), ListenerError> {
        validate_peer(peer, &policy.site_local_cidrs)?;
        match &policy.tls {
            Some(acceptor) => Ok((
                AcceptedStream::Tls(Box::new(acceptor.accept(stream).await?)),
                peer,
            )),
            None => Ok((AcceptedStream::PrivatePlaintext(stream), peer)),
        }
    }

    /// Accept one connection and finish its TLS handshake within a finite bound. Task 5 servers
    /// use this boundary so a peer that connects and then stalls cannot occupy the accept path.
    pub async fn accept_with_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> Result<(AcceptedStream, SocketAddr), ListenerError> {
        if timeout.is_zero() {
            return Err(ListenerError::InvalidAcceptTimeout);
        }
        tokio::time::timeout(timeout, self.accept())
            .await
            .map_err(|_| ListenerError::AcceptTimeout)?
    }

    pub fn serve<C: crate::MonotonicClock>(
        self,
        service: crate::HttpIngestService<C>,
    ) -> io::Result<ServingListener> {
        self.serve_inner(service, false)
    }

    /// Start a viable bound listener with accepting paused. This is used for a different-bind
    /// switchover so the new socket cannot expose a generation before durable publication.
    pub fn serve_paused<C: crate::MonotonicClock>(
        self,
        service: crate::HttpIngestService<C>,
    ) -> io::Result<ServingListener> {
        self.serve_inner(service, true)
    }

    fn serve_inner<C: crate::MonotonicClock>(
        self,
        service: crate::HttpIngestService<C>,
        initially_paused: bool,
    ) -> io::Result<ServingListener> {
        let local_addr = self.local_addr()?;
        let Listener {
            inner,
            policy,
            configured_addr,
        } = self;
        let (shutdown, mut accept_shutdown) = watch::channel(false);
        let (control, mut commands) = mpsc::channel(8);
        let task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            let mut policy = policy;
            let mut paused = initially_paused;
            loop {
                tokio::select! {
                    changed = accept_shutdown.changed() => {
                        if changed.is_err() || *accept_shutdown.borrow() {
                            break;
                        }
                    }
                    command = commands.recv() => {
                        match command {
                            Some(ListenerCommand::Pause { response }) => {
                                paused = true;
                                let _ = response.send(());
                            }
                            Some(ListenerCommand::ReplacePolicy { policy: next, response }) => {
                                if !paused {
                                    let _ = response.send(Err(io::Error::new(
                                        io::ErrorKind::InvalidInput,
                                        "listener must be paused before policy replacement",
                                    )));
                                } else {
                                    let old = std::mem::replace(&mut policy, next);
                                    let _ = response.send(Ok(old));
                                }
                            }
                            Some(ListenerCommand::Resume { response }) => {
                                paused = false;
                                let _ = response.send(());
                            }
                            None => break,
                        }
                    }
                    accepted = inner.accept(), if !paused => {
                        let (stream, peer) = match accepted {
                            Ok(accepted) => accepted,
                            Err(error) => {
                                tracing::error!(error = %error, "HTTP ingress accept loop stopped");
                                break;
                            }
                        };
                        let connection = match service.try_acquire_connection() {
                            Ok(connection) => connection,
                            Err(crate::ServeConnectionError::Busy) => {
                                // No HTTP response is possible before TLS. Closing the raw
                                // stream is the bounded overload behavior for this peer.
                                continue;
                            }
                            Err(error) => {
                                tracing::error!(error = %error, "HTTP ingress connection admission failed");
                                continue;
                            }
                        };
                        let peer_policy = policy.clone();
                        let service = service.clone();
                        let mut connection_shutdown = accept_shutdown.clone();
                        connections.spawn(async move {
                            let negotiated = tokio::time::timeout(
                                TLS_HANDSHAKE_TIMEOUT,
                                Listener::accept_stream(stream, peer, &peer_policy),
                            )
                            .await
                            .map_err(|_| ListenerError::AcceptTimeout)
                            .and_then(|result| result);
                            let (stream, peer) = match negotiated {
                                Ok(accepted) => accepted,
                                Err(error) => {
                                    tracing::debug!(error = %error, peer = %peer, "HTTP ingress peer connection rejected");
                                    return;
                                }
                            };
                            tokio::select! {
                                changed = connection_shutdown.changed() => {
                                    let _ = changed;
                                }
                                result = service.serve_connection_with_permit(stream, peer, connection) => {
                                    if let Err(error) = result {
                                        tracing::debug!(error = %error, peer = %peer, "HTTP ingress connection ended");
                                    }
                                }
                            }
                        });
                    }
                    Some(result) = connections.join_next(), if !connections.is_empty() => {
                        if let Err(error) = result {
                            tracing::error!(error = %error, "HTTP ingress connection task failed");
                        }
                    }
                }
            }
            connections.shutdown().await;
        });
        Ok(ServingListener {
            configured_addr,
            local_addr,
            shutdown,
            control,
            task,
        })
    }
}

pub enum AcceptedStream {
    Tls(Box<tokio_rustls::server::TlsStream<tokio::net::TcpStream>>),
    PrivatePlaintext(tokio::net::TcpStream),
}

impl AsyncRead for AcceptedStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tls(stream) => std::pin::Pin::new(stream).poll_read(cx, buf),
            Self::PrivatePlaintext(stream) => std::pin::Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for AcceptedStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Tls(stream) => std::pin::Pin::new(stream).poll_write(cx, buf),
            Self::PrivatePlaintext(stream) => std::pin::Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tls(stream) => std::pin::Pin::new(stream).poll_flush(cx),
            Self::PrivatePlaintext(stream) => std::pin::Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tls(stream) => std::pin::Pin::new(stream).poll_shutdown(cx),
            Self::PrivatePlaintext(stream) => std::pin::Pin::new(stream).poll_shutdown(cx),
        }
    }
}

fn interface_has_default_route(interface: &str) -> Result<bool, ListenerError> {
    let ipv4 = std::fs::read_to_string("/proc/net/route")?;
    if ipv4.lines().skip(1).any(|line| {
        let mut fields = line.split_whitespace();
        fields.next() == Some(interface) && fields.next() == Some("00000000")
    }) {
        return Ok(true);
    }
    let ipv6 = std::fs::read_to_string("/proc/net/ipv6_route")?;
    Ok(ipv6.lines().any(|line| {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        fields.len() > 9
            && fields[0] == "00000000000000000000000000000000"
            && fields[1] == "00"
            && fields[9] == interface
    }))
}

pub fn validate_peer(peer: SocketAddr, cidrs: &[SiteCidr]) -> Result<(), ListenerError> {
    let peer = normalize_ip(peer.ip());
    validate_private_ip(peer, true)?;
    if cidrs.iter().any(|cidr| cidr_contains(cidr, peer)) {
        Ok(())
    } else {
        Err(ListenerError::PeerOutsideSiteCidr)
    }
}

fn normalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(ip)),
        ip => ip,
    }
}

fn loopback_cidr(ip: IpAddr) -> SiteCidr {
    match normalize_ip(ip) {
        IpAddr::V4(_) => "127.0.0.0/8".parse().expect("loopback CIDR literal"),
        IpAddr::V6(_) => "::1/128".parse().expect("loopback CIDR literal"),
    }
}

fn cidr_contains(cidr: &SiteCidr, ip: IpAddr) -> bool {
    match (cidr, normalize_ip(ip)) {
        (IpNet::V4(cidr), IpAddr::V4(ip)) => cidr.contains(&ip),
        (IpNet::V6(cidr), IpAddr::V6(ip)) => cidr.contains(&ip),
        _ => false,
    }
}

fn validate_cidr(cidr: &SiteCidr, allow_loopback: bool) -> Result<(), ListenerError> {
    let contained = match cidr {
        IpNet::V4(net) => {
            let blocks = [
                ipnet::Ipv4Net::new("10.0.0.0".parse().expect("literal"), 8).expect("literal"),
                ipnet::Ipv4Net::new("172.16.0.0".parse().expect("literal"), 12).expect("literal"),
                ipnet::Ipv4Net::new("192.168.0.0".parse().expect("literal"), 16).expect("literal"),
            ];
            blocks
                .iter()
                .any(|block| block.contains(&net.network()) && block.contains(&net.broadcast()))
                || (allow_loopback
                    && ipnet::Ipv4Net::new("127.0.0.0".parse().expect("literal"), 8)
                        .expect("literal")
                        .contains(&net.network())
                    && net.broadcast().is_loopback())
        }
        IpNet::V6(net) => net.prefix_len() >= 7 && (net.network().segments()[0] & 0xfe00) == 0xfc00,
    };
    contained
        .then_some(())
        .ok_or(ListenerError::InternetCapableExposure)
}

fn validate_private_ip(ip: IpAddr, allow_loopback: bool) -> Result<(), ListenerError> {
    let safe = match normalize_ip(ip) {
        IpAddr::V4(ip) => ip.is_private() || (allow_loopback && ip.is_loopback()),
        IpAddr::V6(ip) => {
            (ip.segments()[0] & 0xfe00) == 0xfc00 || (allow_loopback && ip.is_loopback())
        }
    };
    safe.then_some(()).ok_or(ListenerError::PublicAddress)
}

fn certificate_fingerprint(cert_pem: &[u8]) -> Result<String, ListenerError> {
    let pem = std::str::from_utf8(cert_pem).map_err(|_| ListenerError::CorruptTls)?;
    iotkit_core_ops::fingerprint_of_pem(pem).map_err(|_| ListenerError::CorruptTls)
}

fn build_server_config(cert_pem: &[u8], key_pem: &[u8]) -> Result<ServerConfig, ListenerError> {
    let certs = CertificateDer::pem_slice_iter(cert_pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ListenerError::CorruptTls)?;
    let key = PrivateKeyDer::from_pem_slice(key_pem).map_err(|_| ListenerError::CorruptTls)?;
    ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|_| ListenerError::TlsPairMismatch)
}

#[derive(Debug, thiserror::Error)]
pub enum ListenerError {
    #[error("internet-capable or wildcard exposure is forbidden")]
    InternetCapableExposure,
    #[error("public address is forbidden")]
    PublicAddress,
    #[error("listener interface is not approved")]
    UnapprovedInterface,
    #[error("bind address is not assigned to the approved interface")]
    BindNotOnInterface,
    #[error("site-local CIDR is required")]
    MissingSiteCidr,
    #[error("bind address is outside the site-local CIDR")]
    BindOutsideSiteCidr,
    #[error("accepted peer is outside the site-local CIDR")]
    PeerOutsideSiteCidr,
    #[error("certificate and private key must both be present")]
    IncompleteTls,
    #[error("TLS generation must be positive")]
    InvalidTlsGeneration,
    #[error("TLS certificate is corrupt")]
    CorruptTls,
    #[error("TLS certificate and private key do not match")]
    TlsPairMismatch,
    #[error("TLS certificate fingerprint does not match approved material")]
    TlsFingerprintMismatch,
    #[error("TLS material exceeds the construction limit")]
    TlsMaterialTooLarge,
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("TLS handshake failed")]
    TlsHandshake(#[from] rustls::Error),
    #[error("listener accept/TLS handshake timed out")]
    AcceptTimeout,
    #[error("listener accept timeout must be positive")]
    InvalidAcceptTimeout,
}

#[cfg(test)]
#[path = "../tests/admission.rs"]
mod admission_tests;

#[cfg(test)]
#[path = "../tests/listener_boundary.rs"]
mod listener_boundary_tests;

#[cfg(test)]
#[path = "../tests/e2e.rs"]
mod e2e_tests;
