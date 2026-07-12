//! Safe transport-construction boundary for the HTTP ingest listener.
//!
//! HTTP routes and ingest-domain behavior deliberately do not live here yet. Task 4 only owns
//! exposure classification, TLS material validation, peer checks, and socket construction.

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
use tokio_rustls::TlsAcceptor;

#[cfg(test)]
extern crate self as iotkit_ingest_http;

mod transition;
pub use transition::{ApplyError, ListenerTransition};

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
    internet_default_route: bool,
}

impl ExposureSnapshot {
    /// Reads interface addresses and default-route state from the running OS. Product listener
    /// construction uses this trusted producer; arbitrary snapshots are test-only.
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
            internet_default_route: interface_has_default_route(interface)?,
        })
    }

    #[doc(hidden)]
    pub fn new(
        interface: impl Into<String>,
        addresses: impl IntoIterator<Item = IpAddr>,
        internet_default_route: bool,
    ) -> Self {
        Self {
            interface: interface.into(),
            addresses: addresses.into_iter().map(normalize_ip).collect(),
            internet_default_route,
        }
    }

    #[doc(hidden)]
    pub fn interface(&self) -> &str {
        &self.interface
    }

    #[doc(hidden)]
    pub fn addresses(&self) -> &BTreeSet<IpAddr> {
        &self.addresses
    }

    #[doc(hidden)]
    pub fn internet_default_route(&self) -> bool {
        self.internet_default_route
    }
}

#[derive(Debug, Clone)]
pub struct ValidatedListenerConfig {
    config: ListenerConfig,
    degraded: bool,
    test_only: bool,
}

impl ValidatedListenerConfig {
    pub fn new(config: ListenerConfig, exposure: &ExposureSnapshot) -> Result<Self, ListenerError> {
        Self::validate(config, exposure, false)
    }

    /// Loopback is deliberately available only to crate/integration tests. Product configuration
    /// must name a real private site interface and CIDR.
    pub fn new_for_test(
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
        Ok(Self {
            config,
            degraded,
            test_only: allow_loopback,
        })
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
    config: ValidatedListenerConfig,
    tls: Option<TlsAcceptor>,
}

impl Listener {
    pub async fn bind(config: ValidatedListenerConfig) -> Result<Self, ListenerError> {
        let tls = match config.mode() {
            ListenerMode::Tls(material) => Some(TlsAcceptor::from(material.server_config()?)),
            ListenerMode::PrivatePlaintext => None,
        };
        let socket = socket2::Socket::new(
            match config.bind_addr() {
                SocketAddr::V4(_) => socket2::Domain::IPV4,
                SocketAddr::V6(_) => socket2::Domain::IPV6,
            },
            socket2::Type::STREAM,
            Some(socket2::Protocol::TCP),
        )?;
        socket.set_nonblocking(true)?;
        #[cfg(any(target_os = "linux", target_os = "android"))]
        if !config.test_only {
            socket.bind_device(Some(config.config.interface.as_bytes()))?;
        }
        socket.bind(&config.bind_addr().into())?;
        socket.listen(128)?;
        let inner = tokio::net::TcpListener::from_std(socket.into())?;
        Ok(Self { inner, config, tls })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }

    pub async fn accept(&self) -> Result<(AcceptedStream, SocketAddr), ListenerError> {
        let (stream, peer) = self.inner.accept().await?;
        validate_peer(peer, self.config.site_local_cidrs())?;
        match &self.tls {
            Some(acceptor) => Ok((
                AcceptedStream::Tls(Box::new(acceptor.accept(stream).await?)),
                peer,
            )),
            None => Ok((AcceptedStream::PrivatePlaintext(stream), peer)),
        }
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
}
