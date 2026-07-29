use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
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
use zeroize::Zeroizing;

use crate::{BackupPassphrase, NodeBackupManifest, RecoveryError, SnapshotMode};

const MAGIC: &[u8; 8] = b"IOTKNDB1";
const HEADER_MAX_BYTES: usize = 16 * 1024;
const MANIFEST_MAX_BYTES: usize = 1024 * 1024;
const KEY_BYTES: usize = 32;
const SALT_BYTES: usize = 16;
const NONCE_PREFIX_BYTES: usize = 16;
const NONCE_BYTES: usize = 24;
const TAG_BYTES: usize = 16;
const DEFAULT_KDF_TIME: u32 = 3;
const DEFAULT_KDF_MEMORY_KIB: u32 = 65_536;
const DEFAULT_KDF_PARALLELISM: u32 = 4;
const DEFAULT_CHUNK_SIZE: usize = 256 * 1024;
const MIN_CHUNK_SIZE: usize = 4 * 1024;
const MAX_CHUNK_SIZE: usize = 4 * 1024 * 1024;
const DATA_FLAGS: u8 = 0;
const TERMINAL_FLAGS: u8 = 1;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContainerHeader {
    artifact_kind: String,
    #[serde(deserialize_with = "crate::model::integer::u32")]
    format_version: u32,
    kdf: String,
    salt_b64: String,
    #[serde(deserialize_with = "crate::model::integer::u32")]
    kdf_time: u32,
    #[serde(deserialize_with = "crate::model::integer::u32")]
    kdf_memory_kib: u32,
    #[serde(deserialize_with = "crate::model::integer::u32")]
    kdf_parallelism: u32,
    cipher: String,
    nonce_prefix_b64: String,
    #[serde(deserialize_with = "crate::model::integer::usize")]
    chunk_size: usize,
}

struct ParsedHeader {
    header: ContainerHeader,
    salt: [u8; SALT_BYTES],
    nonce_prefix: [u8; NONCE_PREFIX_BYTES],
    digest: [u8; 32],
}

/// Encrypts a sanitized snapshot into a new Node backup container.
pub fn encrypt_container(
    snapshot: &Path,
    manifest: &NodeBackupManifest,
    passphrase: &BackupPassphrase,
    output: &Path,
) -> Result<(), RecoveryError> {
    validate_passphrase(passphrase)?;
    let mut salt = [0_u8; SALT_BYTES];
    let mut nonce_prefix = [0_u8; NONCE_PREFIX_BYTES];
    getrandom::fill(&mut salt).map_err(|_| RecoveryError::Random)?;
    getrandom::fill(&mut nonce_prefix).map_err(|_| RecoveryError::Random)?;
    encrypt_with_entropy(snapshot, manifest, passphrase, output, salt, nonce_prefix)
}

fn encrypt_with_entropy(
    snapshot: &Path,
    manifest: &NodeBackupManifest,
    passphrase: &BackupPassphrase,
    output: &Path,
    salt: [u8; SALT_BYTES],
    nonce_prefix: [u8; NONCE_PREFIX_BYTES],
) -> Result<(), RecoveryError> {
    let snapshot_file = File::open(snapshot).map_err(|_| RecoveryError::Storage)?;
    encrypt_open_snapshot(
        snapshot_file,
        manifest,
        passphrase,
        output,
        salt,
        nonce_prefix,
    )
}

fn encrypt_open_snapshot(
    mut snapshot_file: File,
    manifest: &NodeBackupManifest,
    passphrase: &BackupPassphrase,
    output: &Path,
    salt: [u8; SALT_BYTES],
    nonce_prefix: [u8; NONCE_PREFIX_BYTES],
) -> Result<(), RecoveryError> {
    validate_manifest(manifest)?;
    verify_snapshot_digest(&mut snapshot_file, manifest)?;
    ensure_absent(output)?;

    let header = ContainerHeader {
        artifact_kind: "iotkit_edge_node_database".into(),
        format_version: 1,
        kdf: "argon2id".into(),
        salt_b64: STANDARD_NO_PAD.encode(salt),
        kdf_time: DEFAULT_KDF_TIME,
        kdf_memory_kib: DEFAULT_KDF_MEMORY_KIB,
        kdf_parallelism: DEFAULT_KDF_PARALLELISM,
        cipher: "xchacha20-poly1305".into(),
        nonce_prefix_b64: STANDARD_NO_PAD.encode(nonce_prefix),
        chunk_size: DEFAULT_CHUNK_SIZE,
    };
    let header_json = serde_json::to_vec(&header).map_err(|_| RecoveryError::ContainerInvalid)?;
    if header_json.is_empty() || header_json.len() > HEADER_MAX_BYTES {
        return Err(RecoveryError::ContainerInvalid);
    }
    let digest = header_digest(&header_json);
    let key = derive_key(passphrase, &salt, &header)?;
    let cipher =
        XChaCha20Poly1305::new_from_slice(key.as_ref()).map_err(|_| RecoveryError::Cryptography)?;

    let manifest_json = serde_json::to_vec(manifest).map_err(|_| RecoveryError::ManifestInvalid)?;
    if manifest_json.is_empty() || manifest_json.len() > MANIFEST_MAX_BYTES {
        return Err(RecoveryError::ManifestInvalid);
    }
    let manifest_len =
        u32::try_from(manifest_json.len()).map_err(|_| RecoveryError::ManifestInvalid)?;
    let mut input = io::Cursor::new(manifest_len.to_be_bytes().to_vec())
        .chain(io::Cursor::new(manifest_json))
        .chain(snapshot_file);

    let mut output_file = private_new_file(output)?;
    let result = (|| {
        output_file
            .write_all(MAGIC)
            .map_err(|_| RecoveryError::Storage)?;
        output_file
            .write_all(&(header_json.len() as u32).to_be_bytes())
            .map_err(|_| RecoveryError::Storage)?;
        output_file
            .write_all(&header_json)
            .map_err(|_| RecoveryError::Storage)?;

        let mut buffer = vec![0_u8; header.chunk_size];
        let mut sequence = 0_u64;
        loop {
            let count = read_chunk(&mut input, &mut buffer).map_err(|_| RecoveryError::Storage)?;
            if count == 0 {
                write_record(
                    &mut output_file,
                    &cipher,
                    &nonce_prefix,
                    &digest,
                    sequence,
                    TERMINAL_FLAGS,
                    &[],
                )?;
                break;
            }
            write_record(
                &mut output_file,
                &cipher,
                &nonce_prefix,
                &digest,
                sequence,
                DATA_FLAGS,
                &buffer[..count],
            )?;
            sequence = sequence
                .checked_add(1)
                .ok_or(RecoveryError::ContainerInvalid)?;
        }
        output_file.sync_all().map_err(|_| RecoveryError::Storage)
    })();
    drop(output_file);
    if result.is_err() {
        let _ = fs::remove_file(output);
    }
    result
}

/// Authenticates every record without creating or writing any plaintext file.
pub fn authenticate_container(
    input: &Path,
    passphrase: &BackupPassphrase,
) -> Result<NodeBackupManifest, RecoveryError> {
    validate_passphrase(passphrase)?;
    let mut file = File::open(input).map_err(|_| RecoveryError::Storage)?;
    let parsed = parse_header(&mut file)?;
    let key = derive_key(passphrase, &parsed.salt, &parsed.header)?;
    let cipher =
        XChaCha20Poly1305::new_from_slice(key.as_ref()).map_err(|_| RecoveryError::Cryptography)?;
    let mut consumer = PlaintextConsumer::new(None, u64::MAX)?;
    consume_records(
        &mut file,
        &cipher,
        &parsed.nonce_prefix,
        &parsed.digest,
        parsed.header.chunk_size,
        &mut consumer,
    )?;
    consumer.finish()
}

/// Authenticates and streams database plaintext into a newly-created output.
pub fn decrypt_container_to_new_file(
    input: &Path,
    passphrase: &BackupPassphrase,
    output: &Path,
    plaintext_capacity_bytes: u64,
) -> Result<NodeBackupManifest, RecoveryError> {
    validate_passphrase(passphrase)?;
    ensure_absent(output)?;
    let mut file = File::open(input).map_err(|_| RecoveryError::Storage)?;
    let parsed = parse_header(&mut file)?;
    let key = derive_key(passphrase, &parsed.salt, &parsed.header)?;
    let cipher =
        XChaCha20Poly1305::new_from_slice(key.as_ref()).map_err(|_| RecoveryError::Cryptography)?;
    let mut consumer = PlaintextConsumer::new(Some(output), plaintext_capacity_bytes)?;
    let records_result = consume_records(
        &mut file,
        &cipher,
        &parsed.nonce_prefix,
        &parsed.digest,
        parsed.header.chunk_size,
        &mut consumer,
    );
    let result = match records_result {
        Ok(()) => consumer.finish().and_then(|manifest| {
            consumer.publish()?;
            Ok(manifest)
        }),
        Err(error) => Err(error),
    };
    if result.is_err() {
        consumer.cleanup();
    }
    result
}

fn parse_header(input: &mut File) -> Result<ParsedHeader, RecoveryError> {
    let mut magic = [0_u8; MAGIC.len()];
    read_exact(input, &mut magic)?;
    if &magic != MAGIC {
        return Err(RecoveryError::ContainerInvalid);
    }
    let mut length_bytes = [0_u8; 4];
    read_exact(input, &mut length_bytes)?;
    let header_length = u32::from_be_bytes(length_bytes) as usize;
    if header_length == 0 || header_length > HEADER_MAX_BYTES {
        return Err(RecoveryError::ContainerInvalid);
    }
    let mut header_json = vec![0_u8; header_length];
    read_exact(input, &mut header_json)?;
    let header: ContainerHeader =
        serde_json::from_slice(&header_json).map_err(|_| RecoveryError::ContainerInvalid)?;
    validate_header(&header)?;
    let salt = decode_16(&header.salt_b64)?;
    let nonce_prefix = decode_16(&header.nonce_prefix_b64)?;
    Ok(ParsedHeader {
        header,
        salt,
        nonce_prefix,
        digest: header_digest_with_length(&header_json),
    })
}

fn validate_header(header: &ContainerHeader) -> Result<(), RecoveryError> {
    if header.artifact_kind != "iotkit_edge_node_database"
        || header.format_version != 1
        || header.kdf != "argon2id"
        || header.cipher != "xchacha20-poly1305"
        || !(1..=10).contains(&header.kdf_time)
        || !(16_384..=262_144).contains(&header.kdf_memory_kib)
        || !(1..=16).contains(&header.kdf_parallelism)
        || !(MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&header.chunk_size)
    {
        return Err(RecoveryError::ContainerInvalid);
    }
    Ok(())
}

fn decode_16(encoded: &str) -> Result<[u8; 16], RecoveryError> {
    let decoded = STANDARD_NO_PAD
        .decode(encoded)
        .map_err(|_| RecoveryError::ContainerInvalid)?;
    decoded
        .try_into()
        .map_err(|_| RecoveryError::ContainerInvalid)
}

fn derive_key(
    passphrase: &BackupPassphrase,
    salt: &[u8; SALT_BYTES],
    header: &ContainerHeader,
) -> Result<Zeroizing<[u8; KEY_BYTES]>, RecoveryError> {
    let parameters = Params::new(
        header.kdf_memory_kib,
        header.kdf_time,
        header.kdf_parallelism,
        Some(KEY_BYTES),
    )
    .map_err(|_| RecoveryError::Cryptography)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, parameters);
    let mut key = Zeroizing::new([0_u8; KEY_BYTES]);
    argon
        .hash_password_into(passphrase.as_bytes(), salt, key.as_mut())
        .map_err(|_| RecoveryError::Cryptography)?;
    Ok(key)
}

fn consume_records(
    input: &mut File,
    cipher: &XChaCha20Poly1305,
    nonce_prefix: &[u8; NONCE_PREFIX_BYTES],
    digest: &[u8; 32],
    chunk_size: usize,
    consumer: &mut PlaintextConsumer<'_>,
) -> Result<(), RecoveryError> {
    let mut sequence = 0_u64;
    loop {
        let mut flags = [0_u8; 1];
        read_exact(input, &mut flags)?;
        if flags[0] != DATA_FLAGS && flags[0] != TERMINAL_FLAGS {
            return Err(RecoveryError::ContainerInvalid);
        }
        let mut length_bytes = [0_u8; 4];
        read_exact(input, &mut length_bytes)?;
        let plaintext_length = u32::from_be_bytes(length_bytes) as usize;
        if flags[0] == TERMINAL_FLAGS {
            if plaintext_length != 0 {
                return Err(RecoveryError::ContainerInvalid);
            }
        } else if plaintext_length == 0 || plaintext_length > chunk_size {
            return Err(RecoveryError::ContainerInvalid);
        }
        let ciphertext_length = plaintext_length
            .checked_add(TAG_BYTES)
            .ok_or(RecoveryError::ContainerInvalid)?;
        let mut ciphertext = vec![0_u8; ciphertext_length];
        read_exact(input, &mut ciphertext)?;
        let nonce = make_nonce(nonce_prefix, sequence);
        let aad = make_aad(digest, sequence, flags[0], &length_bytes);
        let plaintext = cipher
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| RecoveryError::AuthenticationFailed)?;
        if plaintext.len() != plaintext_length {
            return Err(RecoveryError::ContainerInvalid);
        }
        if flags[0] == TERMINAL_FLAGS {
            consumer.ensure_terminal()?;
            let mut trailing = [0_u8; 1];
            if input
                .read(&mut trailing)
                .map_err(|_| RecoveryError::Storage)?
                != 0
            {
                return Err(RecoveryError::ContainerInvalid);
            }
            return Ok(());
        }
        consumer.consume(&plaintext)?;
        sequence = sequence
            .checked_add(1)
            .ok_or(RecoveryError::ContainerInvalid)?;
    }
}

struct StagedOutput {
    destination: PathBuf,
    path: PathBuf,
    file: Option<File>,
    identity: same_file::Handle,
    published: bool,
}

impl StagedOutput {
    fn new(destination: &Path) -> Result<Self, RecoveryError> {
        let parent = destination.parent().unwrap_or_else(|| Path::new("."));
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|_| RecoveryError::Random)?;
        let path = parent.join(format!(".iotkit-node-staging-{}.tmp", hex_digest(random)));
        let file = private_new_file(&path)?;
        let identity = match file
            .try_clone()
            .ok()
            .and_then(|clone| same_file::Handle::from_file(clone).ok())
        {
            Some(identity) => identity,
            None => {
                drop(file);
                let _ = fs::remove_file(&path);
                return Err(RecoveryError::Storage);
            }
        };
        Ok(Self {
            destination: destination.to_owned(),
            path,
            file: Some(file),
            identity,
            published: false,
        })
    }

    fn file_mut(&mut self) -> &mut File {
        self.file.as_mut().expect("staging file remains open")
    }

    fn sync_all(&self) -> Result<(), RecoveryError> {
        self.file
            .as_ref()
            .expect("staging file remains open")
            .sync_all()
            .map_err(|_| RecoveryError::Storage)
    }

    fn publish(mut self) -> Result<(), RecoveryError> {
        drop(self.file.take());
        if !file_identity_matches(&self.path, &self.identity) {
            return Err(RecoveryError::ArtifactPublicationUncertain);
        }
        match publish_new_file(&self.path, &self.destination) {
            Ok(()) => {
                self.published = true;
                if !file_identity_matches(&self.path, &self.identity) {
                    return Err(RecoveryError::ArtifactPublicationUncertain);
                }
                fs::remove_file(&self.path).map_err(|_| RecoveryError::ArtifactPublicationUncertain)
            }
            Err(error) => Err(error),
        }
    }
}

impl Drop for StagedOutput {
    fn drop(&mut self) {
        drop(self.file.take());
        if !self.published && file_identity_matches(&self.path, &self.identity) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn file_identity_matches(path: &Path, expected: &same_file::Handle) -> bool {
    same_file::Handle::from_path(path).is_ok_and(|actual| actual == *expected)
}

struct PlaintextConsumer<'a> {
    output_path: Option<&'a Path>,
    output_capacity: u64,
    staged: Option<StagedOutput>,
    prefix: [u8; 4],
    prefix_len: usize,
    manifest_length: Option<usize>,
    manifest_bytes: Vec<u8>,
    manifest: Option<NodeBackupManifest>,
    database_length: u64,
    database_digest: Sha256,
}

impl<'a> PlaintextConsumer<'a> {
    fn new(output_path: Option<&'a Path>, output_capacity: u64) -> Result<Self, RecoveryError> {
        Ok(Self {
            output_path,
            output_capacity,
            staged: None,
            prefix: [0; 4],
            prefix_len: 0,
            manifest_length: None,
            manifest_bytes: Vec::new(),
            manifest: None,
            database_length: 0,
            database_digest: Sha256::new(),
        })
    }

    fn consume(&mut self, mut bytes: &[u8]) -> Result<(), RecoveryError> {
        while !bytes.is_empty() {
            if self.prefix_len < self.prefix.len() {
                let count = (self.prefix.len() - self.prefix_len).min(bytes.len());
                self.prefix[self.prefix_len..self.prefix_len + count]
                    .copy_from_slice(&bytes[..count]);
                self.prefix_len += count;
                bytes = &bytes[count..];
                if self.prefix_len == self.prefix.len() {
                    let length = u32::from_be_bytes(self.prefix) as usize;
                    if length == 0 || length > MANIFEST_MAX_BYTES {
                        return Err(RecoveryError::ContainerInvalid);
                    }
                    self.manifest_length = Some(length);
                    self.manifest_bytes = Vec::with_capacity(length);
                }
                continue;
            }
            if let Some(length) = self.manifest_length
                && self.manifest_bytes.len() < length
            {
                let count = (length - self.manifest_bytes.len()).min(bytes.len());
                self.manifest_bytes.extend_from_slice(&bytes[..count]);
                bytes = &bytes[count..];
                if self.manifest_bytes.len() == length {
                    let manifest: NodeBackupManifest = serde_json::from_slice(&self.manifest_bytes)
                        .map_err(|_| RecoveryError::ManifestInvalid)?;
                    validate_manifest(&manifest)?;
                    if manifest.database_length > self.output_capacity {
                        return Err(RecoveryError::StorageFull);
                    }
                    self.manifest = Some(manifest);
                    if let Some(path) = self.output_path {
                        self.staged = Some(StagedOutput::new(path)?);
                    }
                }
                continue;
            }
            let manifest = self
                .manifest
                .as_ref()
                .ok_or(RecoveryError::ContainerInvalid)?;
            let count = u64::try_from(bytes.len()).map_err(|_| RecoveryError::ContainerInvalid)?;
            self.database_length = self
                .database_length
                .checked_add(count)
                .ok_or(RecoveryError::ManifestInvalid)?;
            if self.database_length > manifest.database_length {
                return Err(RecoveryError::ManifestInvalid);
            }
            self.database_digest.update(bytes);
            if let Some(staged) = self.staged.as_mut() {
                staged
                    .file_mut()
                    .write_all(bytes)
                    .map_err(|_| RecoveryError::Storage)?;
            }
            bytes = &[];
        }
        Ok(())
    }

    fn ensure_terminal(&self) -> Result<(), RecoveryError> {
        if self.manifest.is_none() {
            return Err(RecoveryError::ManifestInvalid);
        }
        let manifest = self
            .manifest
            .as_ref()
            .ok_or(RecoveryError::ManifestInvalid)?;
        if self.database_length != manifest.database_length {
            return Err(RecoveryError::ManifestInvalid);
        }
        let digest = hex_digest(self.database_digest.clone().finalize());
        if digest != manifest.database_sha256 {
            return Err(RecoveryError::ManifestInvalid);
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<NodeBackupManifest, RecoveryError> {
        self.ensure_terminal()?;
        if let Some(staged) = self.staged.as_mut() {
            staged.sync_all()?;
        }
        self.manifest.take().ok_or(RecoveryError::ManifestInvalid)
    }

    fn publish(&mut self) -> Result<(), RecoveryError> {
        if let Some(staged) = self.staged.take() {
            staged.publish()
        } else {
            Ok(())
        }
    }

    fn cleanup(&mut self) {
        drop(self.staged.take());
    }
}

fn validate_manifest(manifest: &NodeBackupManifest) -> Result<(), RecoveryError> {
    if manifest.artifact_kind != "iotkit-node-backup"
        || manifest.format_version != crate::NODE_BACKUP_FORMAT_VERSION
        || manifest.backup_id.is_empty()
        || manifest.edge_node_id.is_empty()
        || manifest.ledger_epoch.is_empty()
        || manifest.created_at_ms < 0
        || manifest.accepted_cursor < 0
        || manifest.allocation_high_water < 0
        || manifest.accepted_cursor > manifest.allocation_high_water
        || !matches!(manifest.snapshot_mode, SnapshotMode::Online)
        || manifest.shutdown_seal_id.is_some()
        || manifest.schema_version
            != crate::all_edge_node_migrations()
                .last()
                .map_or(0, |migration| migration.version)
        || manifest.database_sha256.len() != 64
        || !manifest
            .database_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(RecoveryError::ManifestInvalid);
    }
    if !valid_identity(&manifest.backup_id)
        || !valid_identity(&manifest.edge_node_id)
        || !valid_identity(&manifest.ledger_epoch)
    {
        return Err(RecoveryError::ManifestInvalid);
    }
    Ok(())
}

fn validate_passphrase(passphrase: &BackupPassphrase) -> Result<(), RecoveryError> {
    if !(12..=1024).contains(&passphrase.char_count()) {
        return Err(RecoveryError::InvalidPassphrase);
    }
    Ok(())
}

fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= 255
        && !value.contains(':')
        && !value.chars().any(char::is_control)
}

fn verify_snapshot_digest(
    reader: &mut File,
    manifest: &NodeBackupManifest,
) -> Result<(), RecoveryError> {
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|_| RecoveryError::Storage)?;
    let mut digest = Sha256::new();
    let mut length = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|_| RecoveryError::Storage)?;
        if count == 0 {
            break;
        }
        length = length
            .checked_add(u64::try_from(count).map_err(|_| RecoveryError::ManifestInvalid)?)
            .ok_or(RecoveryError::ManifestInvalid)?;
        digest.update(&buffer[..count]);
    }
    if length != manifest.database_length
        || hex_digest(digest.finalize()) != manifest.database_sha256
    {
        return Err(RecoveryError::ManifestInvalid);
    }
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|_| RecoveryError::Storage)?;
    Ok(())
}

fn write_record(
    output: &mut File,
    cipher: &XChaCha20Poly1305,
    nonce_prefix: &[u8; NONCE_PREFIX_BYTES],
    digest: &[u8; 32],
    sequence: u64,
    flags: u8,
    plaintext: &[u8],
) -> Result<(), RecoveryError> {
    let plaintext_length =
        u32::try_from(plaintext.len()).map_err(|_| RecoveryError::ContainerInvalid)?;
    let length_bytes = plaintext_length.to_be_bytes();
    let nonce = make_nonce(nonce_prefix, sequence);
    let aad = make_aad(digest, sequence, flags, &length_bytes);
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .map_err(|_| RecoveryError::Cryptography)?;
    if ciphertext.len() != plaintext.len() + TAG_BYTES {
        return Err(RecoveryError::Cryptography);
    }
    output
        .write_all(&[flags])
        .map_err(|_| RecoveryError::Storage)?;
    output
        .write_all(&length_bytes)
        .map_err(|_| RecoveryError::Storage)?;
    output
        .write_all(&ciphertext)
        .map_err(|_| RecoveryError::Storage)
}

fn make_nonce(prefix: &[u8; NONCE_PREFIX_BYTES], sequence: u64) -> [u8; NONCE_BYTES] {
    let mut nonce = [0_u8; NONCE_BYTES];
    nonce[..NONCE_PREFIX_BYTES].copy_from_slice(prefix);
    nonce[NONCE_PREFIX_BYTES..].copy_from_slice(&sequence.to_be_bytes());
    nonce
}

fn make_aad(digest: &[u8; 32], sequence: u64, flags: u8, plaintext_length: &[u8; 4]) -> [u8; 45] {
    let mut aad = [0_u8; 45];
    aad[..32].copy_from_slice(digest);
    aad[32..40].copy_from_slice(&sequence.to_be_bytes());
    aad[40] = flags;
    aad[41..].copy_from_slice(plaintext_length);
    aad
}

fn header_digest(header_json: &[u8]) -> [u8; 32] {
    header_digest_with_length(header_json)
}

fn header_digest_with_length(header_json: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(MAGIC);
    digest.update((header_json.len() as u32).to_be_bytes());
    digest.update(header_json);
    digest.finalize().into()
}

fn read_exact(input: &mut File, bytes: &mut [u8]) -> Result<(), RecoveryError> {
    input.read_exact(bytes).map_err(|error| {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            RecoveryError::ContainerInvalid
        } else {
            RecoveryError::Storage
        }
    })
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

fn ensure_absent(path: &Path) -> Result<(), RecoveryError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(RecoveryError::DestinationExists),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(RecoveryError::Storage),
    }
}

fn private_new_file(path: &Path) -> Result<File, RecoveryError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            RecoveryError::DestinationExists
        } else {
            RecoveryError::Storage
        }
    })
}

fn publish_new_file(staged: &Path, destination: &Path) -> Result<(), RecoveryError> {
    fs::hard_link(staged, destination).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            RecoveryError::DestinationExists
        } else {
            RecoveryError::Storage
        }
    })
}

fn hex_digest(digest: impl AsRef<[u8]>) -> String {
    let mut encoded = String::with_capacity(64);
    for byte in digest.as_ref() {
        encoded.push_str(&format!("{byte:02x}"));
    }
    encoded
}

#[cfg(test)]
#[path = "../tests/unit/container_tests.rs"]
mod tests;
