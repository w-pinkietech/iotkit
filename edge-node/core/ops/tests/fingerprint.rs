use iotkit_core_ops::fingerprint_of_pem;
use rcgen::generate_simple_self_signed;

#[test]
fn fingerprint_of_pem_returns_stable_colon_separated_sha256() {
    let cert = generate_simple_self_signed(vec!["iotkit-edge.local".to_string()]).unwrap();
    let pem = cert.cert.pem();

    let first = fingerprint_of_pem(&pem).unwrap();
    let second = fingerprint_of_pem(&pem).unwrap();

    assert_eq!(first, second);
    assert_eq!(first.len(), 95);
    assert_eq!(first.split(':').count(), 32);
    assert!(first.split(':').all(|part| {
        part.len() == 2
            && part
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    }));
}

#[test]
fn certificate_chain_fingerprints_only_the_leaf_der() {
    let leaf = generate_simple_self_signed(vec!["leaf.local".to_string()]).unwrap();
    let issuer = generate_simple_self_signed(vec!["issuer.local".to_string()]).unwrap();
    let chain = format!("{}{}", leaf.cert.pem(), issuer.cert.pem());

    assert_eq!(
        fingerprint_of_pem(&chain).unwrap(),
        fingerprint_of_pem(&leaf.cert.pem()).unwrap()
    );
}

#[test]
fn malformed_or_non_certificate_pem_is_rejected_consistently() {
    assert!(fingerprint_of_pem("not pem").is_err());
    assert!(
        fingerprint_of_pem("-----BEGIN CERTIFICATE-----\nnot-base64!\n-----END CERTIFICATE-----\n")
            .is_err()
    );
    let leaf = generate_simple_self_signed(vec!["leaf.local".to_string()]).unwrap();
    let malformed_chain = format!(
        "{}-----BEGIN CERTIFICATE-----\nnot-base64!\n-----END CERTIFICATE-----\n",
        leaf.cert.pem()
    );
    assert!(fingerprint_of_pem(&malformed_chain).is_err());
}
