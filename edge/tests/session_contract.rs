use iotkit_edge::auth::{
    csrf::{OriginError, PublicOrigin, validate_request_origin},
    session::{
        ABSOLUTE_SESSION_LIFETIME_MS, IDLE_SESSION_LIFETIME_MS, SessionSecrets, SessionWindow,
    },
};

#[test]
fn session_secrets_have_fixed_entropy_and_only_digests_are_persistable() {
    let secrets = SessionSecrets::generate().expect("generate session secrets");
    assert_eq!(secrets.session_ref().as_str().len(), 5 + 32);
    assert_eq!(secrets.token().expose_secret().len(), 43);
    assert_eq!(secrets.csrf().expose_secret().len(), 43);
    assert_ne!(
        secrets.token().expose_secret(),
        secrets.csrf().expose_secret()
    );
    assert!(secrets.token_digest().matches(secrets.token()));
    assert!(secrets.csrf_digest().matches(secrets.csrf()));
}

#[test]
fn session_window_slides_idle_but_never_absolute_expiry() {
    let issued_at = 1_700_000_000_000;
    let mut window = SessionWindow::issued(issued_at).expect("issue session");
    assert_eq!(
        window.idle_expires_at(),
        issued_at + IDLE_SESSION_LIFETIME_MS
    );
    assert_eq!(
        window.absolute_expires_at(),
        issued_at + ABSOLUTE_SESSION_LIFETIME_MS
    );
    assert!(window.is_active(issued_at));
    assert!(!window.is_active(window.idle_expires_at()));

    window
        .touch(issued_at + 7 * 60 * 60 * 1_000)
        .expect("first active touch");
    window
        .touch(issued_at + 14 * 60 * 60 * 1_000)
        .expect("second active touch");
    window
        .touch(issued_at + 21 * 60 * 60 * 1_000)
        .expect("third active touch");
    let near_absolute = window.absolute_expires_at() - 1_000;
    window.touch(near_absolute).expect("touch active session");
    assert_eq!(window.idle_expires_at(), window.absolute_expires_at());
    assert!(!window.is_active(window.absolute_expires_at()));
}

#[test]
fn request_origin_uses_exact_origin_or_referer_origin() {
    let public = PublicOrigin::parse("https://edge.example").expect("valid public origin");
    assert!(validate_request_origin(&public, Some("https://edge.example"), None).is_ok());
    assert!(
        validate_request_origin(
            &public,
            None,
            Some("https://edge.example/console/accounts?view=active")
        )
        .is_ok()
    );
    assert_eq!(
        validate_request_origin(&public, Some("http://edge.example"), None),
        Err(OriginError::Forbidden)
    );
    assert_eq!(
        validate_request_origin(&public, None, None),
        Err(OriginError::Missing)
    );
    assert_eq!(
        validate_request_origin(
            &public,
            Some("https://edge.example/path?unexpected=1"),
            None
        ),
        Err(OriginError::Forbidden)
    );
    for invalid in [
        "http://edge.example",
        "https://edge.example/path",
        "https://edge.example?query=1",
        "https://edge.example#fragment",
        "ftp://edge.example",
    ] {
        assert!(PublicOrigin::parse(invalid).is_err(), "{invalid}");
    }
    let development =
        PublicOrigin::parse_for_development("http://127.0.0.1:8080").expect("development origin");
    assert!(validate_request_origin(&development, Some("http://127.0.0.1:8080"), None).is_ok());
}
