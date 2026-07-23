use iotkit_edge::auth::{
    password::{Password, hash_password},
    session::SessionSecrets,
};

#[test]
fn credential_debug_output_is_always_redacted() {
    let password = Password::new("correct horse battery staple").expect("valid password");
    let password_debug = format!("{password:?}");
    assert_eq!(password_debug, "Password([REDACTED])");
    assert!(!password_debug.contains("horse"));

    let password_hash = hash_password(&password).expect("hash password");
    let hash_debug = format!("{password_hash:?}");
    assert_eq!(hash_debug, "PasswordHash([REDACTED])");
    assert!(!hash_debug.contains("argon2"));

    let session = SessionSecrets::generate().expect("generate session");
    let session_debug = format!("{session:?}");
    assert!(!session_debug.contains(session.token().expose_secret()));
    assert!(!session_debug.contains(session.csrf().expose_secret()));
    assert!(session_debug.contains("[REDACTED]"));
}
