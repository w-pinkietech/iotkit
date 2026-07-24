use std::fmt;

use argon2::{
    Algorithm, Argon2, Params, PasswordHasher, PasswordVerifier, Version,
    password_hash::{PasswordHash as ParsedPasswordHash, SaltString},
};
use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};

const MEMORY_KIB: u32 = 64 * 1024;
const ITERATIONS: u32 = 3;
const PARALLELISM: u32 = 1;
const SALT_BYTES: usize = 16;
const HASH_BYTES: usize = 32;

#[derive(Clone, PartialEq, Eq)]
pub struct Password(String);

impl Password {
    pub fn new(value: impl Into<String>) -> Result<Self, PasswordError> {
        let value = value.into();
        let length = value.chars().count();
        if !(12..=128).contains(&length) {
            return Err(PasswordError::Policy);
        }
        Ok(Self(value))
    }

    fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    #[must_use]
    pub fn candidate(&self) -> PasswordCandidate {
        PasswordCandidate(self.0.clone())
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Password([REDACTED])")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct PasswordCandidate(String);

impl PasswordCandidate {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl fmt::Debug for PasswordCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PasswordCandidate([REDACTED])")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct PasswordHash(String);

impl PasswordHash {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PasswordHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PasswordHash([REDACTED])")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verification {
    pub matches: bool,
    pub needs_rehash: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum PasswordError {
    #[error("password must contain between 12 and 128 Unicode codepoints")]
    Policy,
    #[error("login ID must be 3 to 64 bytes of lowercase ASCII letters, digits, '.', '_' or '-'")]
    LoginId,
    #[error("password hash encoding is invalid or outside supported bounds")]
    InvalidHash,
    #[error("secure random generation failed")]
    Random,
    #[error("password hashing failed")]
    Hash,
}

pub fn normalize_login_id(login_id: &str) -> Result<String, PasswordError> {
    let normalized = login_id.to_ascii_lowercase();
    if !(3..=64).contains(&normalized.len())
        || !normalized.bytes().all(|value| {
            value.is_ascii_lowercase() || value.is_ascii_digit() || b"._-".contains(&value)
        })
    {
        return Err(PasswordError::LoginId);
    }
    Ok(normalized)
}

pub fn hash_password(password: &Password) -> Result<PasswordHash, PasswordError> {
    let mut salt_bytes = [0_u8; SALT_BYTES];
    getrandom::fill(&mut salt_bytes).map_err(|_| PasswordError::Random)?;
    let salt = SaltString::encode_b64(&salt_bytes).map_err(|_| PasswordError::Hash)?;
    let params = Params::new(MEMORY_KIB, ITERATIONS, PARALLELISM, Some(HASH_BYTES))
        .map_err(|_| PasswordError::Hash)?;
    let encoded = Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password(password.as_bytes(), &salt)
        .map_err(|_| PasswordError::Hash)?
        .to_string();
    Ok(PasswordHash(encoded))
}

pub fn verify_password(
    encoded: &PasswordHash,
    password: &PasswordCandidate,
) -> Result<Verification, PasswordError> {
    let inspected = inspect_hash(encoded.expose_secret())?;
    let parsed =
        ParsedPasswordHash::new(encoded.expose_secret()).map_err(|_| PasswordError::InvalidHash)?;
    let matches = Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok();
    Ok(Verification {
        matches,
        needs_rehash: matches
            && (inspected.memory_kib != MEMORY_KIB
                || inspected.iterations != ITERATIONS
                || inspected.parallelism != PARALLELISM
                || inspected.salt_len != SALT_BYTES
                || inspected.hash_len != HASH_BYTES),
    })
}

struct InspectedHash {
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
    salt_len: usize,
    hash_len: usize,
}

fn inspect_hash(encoded: &str) -> Result<InspectedHash, PasswordError> {
    let parts = encoded.split('$').collect::<Vec<_>>();
    if parts.len() != 6 || !parts[0].is_empty() || parts[1] != "argon2id" || parts[2] != "v=19" {
        return Err(PasswordError::InvalidHash);
    }
    let parameters = parts[3].split(',').collect::<Vec<_>>();
    if parameters.len() != 3 {
        return Err(PasswordError::InvalidHash);
    }
    let memory_kib = parse_parameter(parameters[0], "m=")?;
    let iterations = parse_parameter(parameters[1], "t=")?;
    let parallelism = parse_parameter(parameters[2], "p=")?;
    if !(8 * 1024..=256 * 1024).contains(&memory_kib)
        || !(1..=10).contains(&iterations)
        || !(1..=8).contains(&parallelism)
    {
        return Err(PasswordError::InvalidHash);
    }
    let salt = STANDARD_NO_PAD
        .decode(parts[4])
        .map_err(|_| PasswordError::InvalidHash)?;
    let hash = STANDARD_NO_PAD
        .decode(parts[5])
        .map_err(|_| PasswordError::InvalidHash)?;
    if !(16..=64).contains(&salt.len()) || !(16..=64).contains(&hash.len()) {
        return Err(PasswordError::InvalidHash);
    }
    Ok(InspectedHash {
        memory_kib,
        iterations,
        parallelism,
        salt_len: salt.len(),
        hash_len: hash.len(),
    })
}

fn parse_parameter(value: &str, prefix: &str) -> Result<u32, PasswordError> {
    value
        .strip_prefix(prefix)
        .ok_or(PasswordError::InvalidHash)?
        .parse()
        .map_err(|_| PasswordError::InvalidHash)
}
