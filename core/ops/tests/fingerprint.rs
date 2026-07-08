use iotkit_core_ops::fingerprint_of_pem;
use rcgen::generate_simple_self_signed;

#[test]
fn fingerprint_of_pem_returns_stable_colon_separated_sha256() {
    let cert = generate_simple_self_signed(vec!["iotkit-gateway.local".to_string()]).unwrap();
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
