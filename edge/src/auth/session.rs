use std::fmt;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub const IDLE_SESSION_LIFETIME_MS: i64 = 8 * 60 * 60 * 1_000;
pub const ABSOLUTE_SESSION_LIFETIME_MS: i64 = 24 * 60 * 60 * 1_000;

macro_rules! secret {
    ($name:ident) => {
        #[derive(Clone, PartialEq, Eq)]
        pub struct $name(String);

        impl $name {
            #[must_use]
            pub fn expose_secret(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($name), "([REDACTED])"))
            }
        }
    };
}

secret!(SessionToken);
secret!(CsrfToken);

#[derive(Clone, PartialEq, Eq)]
pub struct SessionRef(String);

impl SessionRef {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SessionRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionRef([REDACTED])")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SecretDigest([u8; 32]);

impl SecretDigest {
    #[must_use]
    pub(crate) fn from_digest(value: [u8; 32]) -> Self {
        Self(value)
    }

    #[must_use]
    pub fn from_secret(value: &str) -> Self {
        Self(Sha256::digest(value.as_bytes()).into())
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    #[must_use]
    pub fn matches_session(&self, token: &SessionToken) -> bool {
        self.matches_bytes(token.expose_secret())
    }

    #[must_use]
    pub fn matches_csrf(&self, token: &CsrfToken) -> bool {
        self.matches_bytes(token.expose_secret())
    }

    #[must_use]
    pub fn matches<T: SessionSecret>(&self, value: &T) -> bool {
        self.matches_bytes(value.secret_text())
    }

    fn matches_bytes(&self, value: &str) -> bool {
        let actual: [u8; 32] = Sha256::digest(value.as_bytes()).into();
        bool::from(self.0.ct_eq(&actual))
    }
}

impl fmt::Debug for SecretDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretDigest([REDACTED])")
    }
}

pub trait SessionSecret {
    fn secret_text(&self) -> &str;
}

impl SessionSecret for SessionToken {
    fn secret_text(&self) -> &str {
        self.expose_secret()
    }
}

impl SessionSecret for CsrfToken {
    fn secret_text(&self) -> &str {
        self.expose_secret()
    }
}

pub struct SessionSecrets {
    session_ref: SessionRef,
    token: SessionToken,
    csrf: CsrfToken,
}

impl SessionSecrets {
    pub fn generate() -> Result<Self, SessionError> {
        let mut token = [0_u8; 32];
        let mut csrf = [0_u8; 32];
        let mut reference = [0_u8; 16];
        getrandom::fill(&mut token).map_err(|_| SessionError::Random)?;
        getrandom::fill(&mut csrf).map_err(|_| SessionError::Random)?;
        getrandom::fill(&mut reference).map_err(|_| SessionError::Random)?;
        Ok(Self {
            session_ref: SessionRef(format!("sess_{}", hex(&reference))),
            token: SessionToken(URL_SAFE_NO_PAD.encode(token)),
            csrf: CsrfToken(URL_SAFE_NO_PAD.encode(csrf)),
        })
    }

    #[must_use]
    pub fn session_ref(&self) -> &SessionRef {
        &self.session_ref
    }

    #[must_use]
    pub fn token(&self) -> &SessionToken {
        &self.token
    }

    #[must_use]
    pub fn csrf(&self) -> &CsrfToken {
        &self.csrf
    }

    #[must_use]
    pub fn token_digest(&self) -> SecretDigest {
        SecretDigest::from_secret(self.token.expose_secret())
    }

    #[must_use]
    pub fn csrf_digest(&self) -> SecretDigest {
        SecretDigest::from_secret(self.csrf.expose_secret())
    }
}

impl fmt::Debug for SessionSecrets {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionSecrets")
            .field("session_ref", &self.session_ref)
            .field("token", &"[REDACTED]")
            .field("csrf", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionWindow {
    issued_at: i64,
    last_seen_at: i64,
    idle_expires_at: i64,
    absolute_expires_at: i64,
}

impl SessionWindow {
    pub fn issued(issued_at: i64) -> Result<Self, SessionError> {
        if issued_at < 0 {
            return Err(SessionError::InvalidTime);
        }
        Ok(Self {
            issued_at,
            last_seen_at: issued_at,
            idle_expires_at: issued_at
                .checked_add(IDLE_SESSION_LIFETIME_MS)
                .ok_or(SessionError::InvalidTime)?,
            absolute_expires_at: issued_at
                .checked_add(ABSOLUTE_SESSION_LIFETIME_MS)
                .ok_or(SessionError::InvalidTime)?,
        })
    }

    #[must_use]
    pub fn issued_at(&self) -> i64 {
        self.issued_at
    }

    #[must_use]
    pub fn last_seen_at(&self) -> i64 {
        self.last_seen_at
    }

    #[must_use]
    pub fn idle_expires_at(&self) -> i64 {
        self.idle_expires_at
    }

    #[must_use]
    pub fn absolute_expires_at(&self) -> i64 {
        self.absolute_expires_at
    }

    #[must_use]
    pub fn is_active(&self, now: i64) -> bool {
        now >= self.issued_at && now < self.idle_expires_at && now < self.absolute_expires_at
    }

    pub fn touch(&mut self, now: i64) -> Result<(), SessionError> {
        if !self.is_active(now) {
            return Err(SessionError::Expired);
        }
        self.last_seen_at = now;
        self.idle_expires_at = now
            .checked_add(IDLE_SESSION_LIFETIME_MS)
            .ok_or(SessionError::InvalidTime)?
            .min(self.absolute_expires_at);
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("secure random generation failed")]
    Random,
    #[error("session timestamp is invalid")]
    InvalidTime,
    #[error("session is expired")]
    Expired,
}

fn hex(value: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value {
        encoded.push(DIGITS[(byte >> 4) as usize] as char);
        encoded.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    encoded
}
