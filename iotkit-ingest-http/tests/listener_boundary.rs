use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use crate::{
    ExposureSnapshot, ListenerConfig, ListenerMode, LocalIngressCidr, TlsMaterial,
    ValidatedListenerConfig, validate_peer,
};
use rcgen::{CertificateParams, KeyPair};
use rustls::pki_types::pem::PemObject;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

fn private_config(bind: SocketAddr, cidr: &str, mode: ListenerMode) -> ListenerConfig {
    ListenerConfig {
        bind,
        interface: "eth0".into(),
        local_ingress_cidrs: vec![cidr.parse::<LocalIngressCidr>().unwrap()],
        mode,
    }
}

#[test]
fn exposure_validation_rejects_public_wildcard_route_interface_and_cidr() {
    let safe = ExposureSnapshot::new("eth0", ["192.168.10.20".parse().unwrap()], false);
    for config in [
        private_config(
            "0.0.0.0:0".parse().unwrap(),
            "192.168.10.0/24",
            ListenerMode::PrivatePlaintext,
        ),
        private_config(
            "8.8.8.8:0".parse().unwrap(),
            "8.8.8.0/24",
            ListenerMode::PrivatePlaintext,
        ),
        private_config(
            "192.168.10.20:0".parse().unwrap(),
            "0.0.0.0/0",
            ListenerMode::PrivatePlaintext,
        ),
        private_config(
            "192.168.10.20:0".parse().unwrap(),
            "192.168.0.0/15",
            ListenerMode::PrivatePlaintext,
        ),
    ] {
        assert!(ValidatedListenerConfig::new(config, &safe).is_err());
    }

    let wrong_interface = ExposureSnapshot::new("wlan0", ["192.168.10.20".parse().unwrap()], false);
    assert!(
        ValidatedListenerConfig::new(
            private_config(
                "192.168.10.20:0".parse().unwrap(),
                "192.168.10.0/24",
                ListenerMode::PrivatePlaintext
            ),
            &wrong_interface,
        )
        .is_err()
    );

    let default_route = ExposureSnapshot::new("eth0", ["192.168.10.20".parse().unwrap()], true);
    assert!(
        ValidatedListenerConfig::new(
            private_config(
                "192.168.10.20:0".parse().unwrap(),
                "192.168.10.0/24",
                ListenerMode::PrivatePlaintext
            ),
            &default_route,
        )
        .is_ok(),
        "a specific approved private bind remains locally confined when its interface owns the host default route"
    );
}

#[test]
fn mapped_and_ipv6_addresses_cannot_bypass_peer_classification() {
    let cidrs = vec!["192.168.10.0/24".parse().unwrap()];
    assert!(validate_peer("192.168.10.9:1".parse().unwrap(), &cidrs).is_ok());
    assert!(
        validate_peer(
            SocketAddr::new(
                IpAddr::V6(Ipv4Addr::new(192, 168, 10, 9).to_ipv6_mapped()),
                1
            ),
            &cidrs
        )
        .is_ok()
    );
    assert!(
        validate_peer(
            SocketAddr::new(IpAddr::V6(Ipv4Addr::new(8, 8, 8, 8).to_ipv6_mapped()), 1),
            &cidrs
        )
        .is_err()
    );
    assert!(validate_peer(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 1), &cidrs).is_err());
}

#[test]
fn private_plaintext_is_degraded_and_never_valid_on_unsafe_exposure() {
    let exposure = ExposureSnapshot::new("eth0", ["10.1.2.3".parse().unwrap()], false);
    let validated = ValidatedListenerConfig::new(
        private_config(
            "10.1.2.3:0".parse().unwrap(),
            "10.0.0.0/8",
            ListenerMode::PrivatePlaintext,
        ),
        &exposure,
    )
    .unwrap();
    assert!(validated.is_degraded());
    assert_eq!(validated.warning(), Some("private_plaintext"));
}

#[test]
fn trusted_os_inventory_producer_observes_the_named_interface() {
    let exposure = match ExposureSnapshot::from_os("lo") {
        Ok(exposure) => exposure,
        Err(crate::ListenerError::Io(error))
            if error.kind() == std::io::ErrorKind::PermissionDenied =>
        {
            eprintln!("ENVIRONMENTAL SKIP: sandbox denied OS interface inventory");
            return;
        }
        Err(error) => panic!("test host must expose loopback: {error}"),
    };
    assert_eq!(exposure.interface(), "lo");
    assert!(exposure.addresses().iter().any(IpAddr::is_loopback));

    let mut config = private_config(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.0/8",
        ListenerMode::PrivatePlaintext,
    );
    config.interface = "lo".into();
    ValidatedListenerConfig::new_for_test(config, &exposure)
        .unwrap_or_else(|error| panic!("specific loopback inventory must validate: {error}"));
}

#[tokio::test]
async fn safe_loopback_listener_can_be_constructed_directly_for_tests() {
    let exposure = ExposureSnapshot::new("lo", [IpAddr::V4(Ipv4Addr::LOCALHOST)], false);
    let mut config = private_config(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.0/8",
        ListenerMode::PrivatePlaintext,
    );
    config.interface = "lo".into();
    let validated = ValidatedListenerConfig::new_for_test(config, &exposure).unwrap();
    let listener = match crate::Listener::bind(validated).await {
        Ok(listener) => listener,
        Err(crate::ListenerError::Io(error))
            if error.kind() == std::io::ErrorKind::PermissionDenied =>
        {
            eprintln!("ENVIRONMENTAL SKIP: sandbox denied loopback bind; parent rerun required");
            return;
        }
        Err(error) => panic!("safe loopback listener failed: {error}"),
    };
    assert!(listener.local_addr().unwrap().ip().is_loopback());
}

fn tls_pair(name: &str) -> (Vec<u8>, Vec<u8>, String) {
    let key = KeyPair::generate().unwrap();
    let cert = CertificateParams::new(vec![name.into()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let pem = cert.pem();
    let der = cert.der();
    let fingerprint = Sha256::digest(der.as_ref())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(":");
    (
        pem.into_bytes(),
        key.serialize_pem().into_bytes(),
        fingerprint,
    )
}

#[test]
fn missing_corrupt_mismatched_and_unapproved_tls_fail_closed() {
    let (cert_a, key_a, fingerprint_a) = tls_pair("a.test");
    let (_cert_b, key_b, _) = tls_pair("b.test");
    assert!(TlsMaterial::validate(Vec::new(), key_a.clone(), &fingerprint_a, 1).is_err());
    assert!(TlsMaterial::validate(b"bad".to_vec(), b"bad".to_vec(), &fingerprint_a, 1).is_err());
    assert!(TlsMaterial::validate(cert_a.clone(), key_b, &fingerprint_a, 1).is_err());
    assert!(TlsMaterial::validate(cert_a.clone(), key_a.clone(), "sha256:unapproved", 1).is_err());
    let material = TlsMaterial::validate(cert_a, key_a, &fingerprint_a, 1).unwrap();
    assert_eq!(material.fingerprint(), fingerprint_a);
    assert_eq!(material.generation(), 1);
}

#[test]
fn certificate_chain_uses_the_shared_leaf_fingerprint_and_remains_applicable() {
    let (leaf, key, fingerprint) = tls_pair("leaf.test");
    let (issuer, _, _) = tls_pair("issuer.test");
    let mut chain = leaf;
    chain.extend_from_slice(&issuer);

    let material = TlsMaterial::validate(chain, key, &fingerprint, 1).unwrap();
    assert_eq!(material.fingerprint(), fingerprint);
    material.server_config().unwrap();
}

#[tokio::test]
async fn tls_listener_handshakes_before_exposing_a_stream_and_rejects_plaintext() {
    let (cert, key, fingerprint) = tls_pair("localhost");
    let material = TlsMaterial::validate(cert.clone(), key, &fingerprint, 1).unwrap();
    let exposure = ExposureSnapshot::new("lo", [IpAddr::V4(Ipv4Addr::LOCALHOST)], false);
    let mut config = private_config(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.0/8",
        ListenerMode::Tls(material),
    );
    config.interface = "lo".into();
    let listener = match crate::Listener::bind(
        ValidatedListenerConfig::new_for_test(config, &exposure).unwrap(),
    )
    .await
    {
        Ok(listener) => std::sync::Arc::new(listener),
        Err(crate::ListenerError::Io(error))
            if error.kind() == std::io::ErrorKind::PermissionDenied =>
        {
            eprintln!("ENVIRONMENTAL SKIP: sandbox denied loopback bind; parent rerun required");
            return;
        }
        Err(error) => panic!("TLS listener failed: {error}"),
    };

    let plaintext_listener = listener.clone();
    let plaintext_accept = tokio::spawn(async move { plaintext_listener.accept().await });
    let mut raw = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    raw.write_all(b"GET / HTTP/1.0\r\n\r\n").await.unwrap();
    raw.shutdown().await.unwrap();
    assert!(plaintext_accept.await.unwrap().is_err());

    let tls_listener = listener.clone();
    let tls_accept = tokio::spawn(async move { tls_listener.accept().await });
    let mut roots = rustls::RootCertStore::empty();
    let cert_der = rustls::pki_types::CertificateDer::pem_slice_iter(&cert)
        .next()
        .unwrap()
        .unwrap();
    roots.add(cert_der).unwrap();
    let client = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(client));
    let raw = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let _tls = connector
        .connect("localhost".try_into().unwrap(), raw)
        .await
        .unwrap();
    let (accepted, _) = tls_accept.await.unwrap().unwrap();
    assert!(matches!(accepted, crate::AcceptedStream::Tls(_)));
}
