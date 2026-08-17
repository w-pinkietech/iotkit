use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    mem::MaybeUninit,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use argon2::{Algorithm, Argon2, Params, Version};
use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::BackupError;

const MAGIC: &[u8; 8] = b"IOTKBKP1";
const FORMAT_VERSION: u32 = 1;
const CHUNK_SIZE: usize = 256 * 1024;
const MAX_HEADER_SIZE: usize = 64 * 1024;
const MAX_CHUNK_SIZE: usize = 4 * 1024 * 1024;
const KEY_BYTES: usize = 32;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    format_version: u32,
    kdf: String,
    salt: String,
    kdf_time: u32,
    kdf_memory_kib: u32,
    kdf_threads: u32,
    cipher: String,
    nonce_prefix: String,
    chunk_size: usize,
}

pub(super) fn encrypt(
    destination: &Path,
    manifest_json: &[u8],
    payload: &Path,
    passphrase: &str,
) -> Result<(), BackupError> {
    ensure_absent(destination)?;
    let salt = random_bytes::<16>()?;
    let nonce_prefix = random_bytes::<16>()?;
    let header = Header {
        format_version: FORMAT_VERSION,
        kdf: "argon2id".into(),
        salt: STANDARD_NO_PAD.encode(salt),
        kdf_time: 3,
        kdf_memory_kib: 64 * 1024,
        kdf_threads: 4,
        cipher: "xchacha20-poly1305".into(),
        nonce_prefix: STANDARD_NO_PAD.encode(nonce_prefix),
        chunk_size: CHUNK_SIZE,
    };
    let header_json = serde_json::to_vec(&header)?;
    let key = derive_key(passphrase, &salt, &header)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    let header_digest = header_digest(&header_json);

    let temporary = temporary_sibling(destination, "backup")?;
    let result = (|| {
        let mut output = private_new_file(&temporary)?;
        output.write_all(MAGIC)?;
        write_u32(&mut output, header_json.len())?;
        output.write_all(&header_json)?;

        let manifest_len = u32::try_from(manifest_json.len())
            .map_err(|_| BackupError::InvalidManifest)?
            .to_be_bytes();
        let mut input = io::Cursor::new(manifest_len)
            .chain(io::Cursor::new(manifest_json))
            .chain(File::open(payload)?);
        let mut buffer = vec![0_u8; CHUNK_SIZE];
        let mut sequence = 0_u64;
        loop {
            let count = read_chunk(&mut input, &mut buffer)?;
            if count == 0 {
                write_encrypted_chunk(
                    &mut output,
                    &cipher,
                    &nonce_prefix,
                    &header_digest,
                    sequence,
                    &[1],
                )?;
                break;
            }
            let mut plain = Vec::with_capacity(count + 1);
            plain.push(0);
            plain.extend_from_slice(&buffer[..count]);
            write_encrypted_chunk(
                &mut output,
                &cipher,
                &nonce_prefix,
                &header_digest,
                sequence,
                &plain,
            )?;
            sequence = sequence
                .checked_add(1)
                .ok_or(BackupError::InvalidContainer)?;
        }
        output.sync_all()?;
        drop(output);
        publish_new_file(&temporary, destination)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn random_bytes<const N: usize>() -> Result<[u8; N], BackupError> {
    let mut bytes = [MaybeUninit::uninit(); N];
    let initialized = getrandom::fill_uninit(&mut bytes).map_err(|_| BackupError::Cryptography)?;
    <[u8; N]>::try_from(initialized).map_err(|_| BackupError::Cryptography)
}

pub(super) fn decrypt(
    source: &Path,
    destination: &Path,
    passphrase: &str,
) -> Result<(), BackupError> {
    let mut input = File::open(source)?;
    let mut magic = [0_u8; 8];
    input.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(BackupError::InvalidContainer);
    }
    let header_length = read_u32(&mut input)? as usize;
    if header_length == 0 || header_length > MAX_HEADER_SIZE {
        return Err(BackupError::InvalidContainer);
    }
    let mut header_json = vec![0_u8; header_length];
    input.read_exact(&mut header_json)?;
    let header: Header =
        serde_json::from_slice(&header_json).map_err(|_| BackupError::InvalidContainer)?;
    validate_header(&header)?;
    let salt = decode_16(&header.salt)?;
    let nonce_prefix = decode_16(&header.nonce_prefix)?;
    let key = derive_key(passphrase, &salt, &header)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    let digest = header_digest(&header_json);

    let mut output = private_new_file(destination)?;
    let result = (|| {
        for sequence in 0_u64.. {
            let ciphertext_length = read_u32(&mut input)? as usize;
            if ciphertext_length < 17 || ciphertext_length > header.chunk_size + 17 {
                return Err(BackupError::InvalidContainer);
            }
            let mut ciphertext = vec![0_u8; ciphertext_length];
            input.read_exact(&mut ciphertext)?;
            let nonce = nonce(&nonce_prefix, sequence);
            let aad = aad(&digest, sequence);
            let plain = cipher
                .decrypt(
                    XNonce::from_slice(&nonce),
                    Payload {
                        msg: &ciphertext,
                        aad: &aad,
                    },
                )
                .map_err(|_| BackupError::Authentication)?;
            match plain.first() {
                Some(0) => output.write_all(&plain[1..])?,
                Some(1) if plain.len() == 1 => {
                    let mut trailing = [0_u8; 1];
                    if input.read(&mut trailing)? != 0 {
                        return Err(BackupError::InvalidContainer);
                    }
                    output.sync_all()?;
                    return Ok(());
                }
                _ => return Err(BackupError::InvalidContainer),
            }
        }
        unreachable!()
    })();
    drop(output);
    if result.is_err() {
        let _ = fs::remove_file(destination);
    }
    result
}

fn validate_header(header: &Header) -> Result<(), BackupError> {
    if header.format_version != FORMAT_VERSION
        || header.kdf != "argon2id"
        || header.cipher != "xchacha20-poly1305"
        || !(1..=10).contains(&header.kdf_time)
        || !(16 * 1024..=256 * 1024).contains(&header.kdf_memory_kib)
        || !(1..=16).contains(&header.kdf_threads)
        || !(4096..=MAX_CHUNK_SIZE).contains(&header.chunk_size)
    {
        return Err(BackupError::InvalidContainer);
    }
    Ok(())
}

fn derive_key(
    passphrase: &str,
    salt: &[u8; 16],
    header: &Header,
) -> Result<[u8; KEY_BYTES], BackupError> {
    let parameters = Params::new(
        header.kdf_memory_kib,
        header.kdf_time,
        header.kdf_threads,
        Some(KEY_BYTES),
    )
    .map_err(|_| BackupError::Cryptography)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, parameters);
    let mut key = [0_u8; KEY_BYTES];
    argon
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|_| BackupError::Cryptography)?;
    Ok(key)
}

fn write_encrypted_chunk(
    output: &mut File,
    cipher: &XChaCha20Poly1305,
    prefix: &[u8; 16],
    digest: &[u8; 32],
    sequence: u64,
    plain: &[u8],
) -> Result<(), BackupError> {
    let nonce = nonce(prefix, sequence);
    let aad = aad(digest, sequence);
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plain,
                aad: &aad,
            },
        )
        .map_err(|_| BackupError::Cryptography)?;
    write_u32(output, ciphertext.len())?;
    output.write_all(&ciphertext)?;
    Ok(())
}

fn nonce(prefix: &[u8; 16], sequence: u64) -> [u8; 24] {
    let mut nonce = [0_u8; 24];
    nonce[..16].copy_from_slice(prefix);
    nonce[16..].copy_from_slice(&sequence.to_be_bytes());
    nonce
}

fn aad(digest: &[u8; 32], sequence: u64) -> [u8; 40] {
    let mut aad = [0_u8; 40];
    aad[..32].copy_from_slice(digest);
    aad[32..].copy_from_slice(&sequence.to_be_bytes());
    aad
}

fn header_digest(header_json: &[u8]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(MAGIC);
    hash.update(header_json);
    hash.finalize().into()
}

fn decode_16(encoded: &str) -> Result<[u8; 16], BackupError> {
    let decoded = STANDARD_NO_PAD
        .decode(encoded)
        .map_err(|_| BackupError::InvalidContainer)?;
    decoded
        .try_into()
        .map_err(|_| BackupError::InvalidContainer)
}

fn read_chunk(input: &mut impl Read, buffer: &mut [u8]) -> io::Result<usize> {
    let mut count = 0;
    while count < buffer.len() {
        match input.read(&mut buffer[count..])? {
            0 => break,
            read => count += read,
        }
    }
    Ok(count)
}

fn write_u32(output: &mut impl Write, value: usize) -> Result<(), BackupError> {
    let value = u32::try_from(value).map_err(|_| BackupError::InvalidContainer)?;
    output.write_all(&value.to_be_bytes())?;
    Ok(())
}

fn read_u32(input: &mut impl Read) -> Result<u32, BackupError> {
    let mut encoded = [0_u8; 4];
    input
        .read_exact(&mut encoded)
        .map_err(|_| BackupError::InvalidContainer)?;
    Ok(u32::from_be_bytes(encoded))
}

pub(super) fn ensure_absent(path: &Path) -> Result<(), BackupError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(BackupError::DestinationExists),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn private_new_file(path: &Path) -> Result<File, BackupError> {
    Ok(OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?)
}

pub(super) fn publish_new_file(temporary: &Path, destination: &Path) -> Result<(), BackupError> {
    ensure_absent(destination)?;
    fs::hard_link(temporary, destination).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            BackupError::DestinationExists
        } else {
            error.into()
        }
    })?;
    fs::remove_file(temporary)?;
    sync_parent(destination)?;
    Ok(())
}

pub(super) fn temporary_sibling(destination: &Path, purpose: &str) -> Result<PathBuf, BackupError> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    Ok(parent.join(format!(".iotkit-edge-{purpose}-{}.tmp", Uuid::new_v4())))
}

pub(super) fn sync_parent(path: &Path) -> Result<(), BackupError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    File::open(parent)?.sync_all()?;
    Ok(())
}

pub(super) fn protect_directory(path: &Path) -> Result<(), BackupError> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}
