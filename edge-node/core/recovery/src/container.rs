use std::{
    fmt,
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

#[cfg(target_os = "linux")]
use std::ffi::CString;
#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStrExt;

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
#[allow(dead_code)]
const DEFAULT_KDF_TIME: u32 = 3;
#[allow(dead_code)]
const DEFAULT_KDF_MEMORY_KIB: u32 = 65_536;
#[allow(dead_code)]
const DEFAULT_KDF_PARALLELISM: u32 = 4;
#[allow(dead_code)]
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
    snapshot_file: File,
    manifest: &NodeBackupManifest,
    passphrase: &BackupPassphrase,
    output: &Path,
    salt: [u8; SALT_BYTES],
    nonce_prefix: [u8; NONCE_PREFIX_BYTES],
) -> Result<(), RecoveryError> {
    encrypt_snapshot_reader(
        snapshot_file,
        manifest,
        passphrase,
        output,
        salt,
        nonce_prefix,
    )
}

fn encrypt_snapshot_reader(
    snapshot_reader: impl Read,
    manifest: &NodeBackupManifest,
    passphrase: &BackupPassphrase,
    output: &Path,
    salt: [u8; SALT_BYTES],
    nonce_prefix: [u8; NONCE_PREFIX_BYTES],
) -> Result<(), RecoveryError> {
    validate_manifest(manifest)?;
    #[cfg(target_os = "linux")]
    {
        let mut output_file = LinuxEncryptedOutput::new(output)?;
        encrypt_snapshot_contents(
            snapshot_reader,
            manifest,
            passphrase,
            salt,
            nonce_prefix,
            &mut output_file,
        )?;
        output_file.publish()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (
            snapshot_reader,
            manifest,
            passphrase,
            output,
            salt,
            nonce_prefix,
        );
        Err(RecoveryError::PlatformUnsupported)
    }
}

#[allow(dead_code)]
trait EncryptionOutput: Write {
    fn sync_file(&mut self) -> io::Result<()>;
}

#[allow(dead_code)]
fn encrypt_snapshot_contents(
    snapshot_reader: impl Read,
    manifest: &NodeBackupManifest,
    passphrase: &BackupPassphrase,
    salt: [u8; SALT_BYTES],
    nonce_prefix: [u8; NONCE_PREFIX_BYTES],
    output_file: &mut impl EncryptionOutput,
) -> Result<(), RecoveryError> {
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
    let manifest_prefix = manifest_len
        .to_be_bytes()
        .into_iter()
        .chain(manifest_json.iter().copied())
        .collect::<Vec<_>>();
    let mut input = io::Cursor::new(manifest_prefix.clone()).chain(snapshot_reader);
    let mut prefix_remaining = manifest_prefix.len();
    let mut database_length = 0_u64;
    let mut database_digest = Sha256::new();

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
            if prefix_remaining != 0
                || database_length != manifest.database_length
                || hex_digest(database_digest.clone().finalize()) != manifest.database_sha256
            {
                return Err(RecoveryError::ManifestInvalid);
            }
            write_record(
                output_file,
                &cipher,
                &nonce_prefix,
                &digest,
                sequence,
                TERMINAL_FLAGS,
                &[],
            )?;
            break;
        }
        let prefix_count = prefix_remaining.min(count);
        prefix_remaining -= prefix_count;
        let database = &buffer[prefix_count..count];
        let database_count =
            u64::try_from(database.len()).map_err(|_| RecoveryError::ManifestInvalid)?;
        database_length = database_length
            .checked_add(database_count)
            .ok_or(RecoveryError::ManifestInvalid)?;
        if database_length > manifest.database_length {
            return Err(RecoveryError::ManifestInvalid);
        }
        database_digest.update(database);
        write_record(
            output_file,
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
    output_file.sync_file().map_err(|_| RecoveryError::Storage)
}

#[cfg(target_os = "linux")]
struct LinuxEncryptedOutput {
    parent: File,
    file: File,
    name: CString,
    ops: LinuxOutputOps,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
struct LinuxOutputOps {
    write: fn(&mut File, &[u8]) -> io::Result<usize>,
    sync_file: fn(&File) -> io::Result<()>,
    link: fn(&File, &File, &CString) -> io::Result<()>,
    sync_directory: fn(&File) -> io::Result<()>,
}

#[cfg(target_os = "linux")]
impl LinuxOutputOps {
    fn system() -> Self {
        Self {
            write: system_write,
            sync_file: system_sync_file,
            link: system_link,
            sync_directory: system_sync_directory,
        }
    }
}

#[cfg(target_os = "linux")]
fn system_write(file: &mut File, bytes: &[u8]) -> io::Result<usize> {
    file.write(bytes)
}

#[cfg(target_os = "linux")]
fn system_sync_file(file: &File) -> io::Result<()> {
    file.sync_all()
}

#[cfg(target_os = "linux")]
fn system_link(file: &File, parent: &File, name: &CString) -> io::Result<()> {
    let result = unsafe {
        libc::linkat(
            file.as_raw_fd(),
            c"".as_ptr(),
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::AT_EMPTY_PATH,
        )
    };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn system_sync_directory(parent: &File) -> io::Result<()> {
    let result = unsafe { libc::fsync(parent.as_raw_fd()) };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
impl LinuxEncryptedOutput {
    fn new(output: &Path) -> Result<Self, RecoveryError> {
        Self::new_with_ops(output, LinuxOutputOps::system())
    }

    fn new_with_ops(output: &Path, ops: LinuxOutputOps) -> Result<Self, RecoveryError> {
        let parent_path = output.parent().unwrap_or_else(|| Path::new("."));
        let name = output
            .file_name()
            .filter(|name| !name.is_empty() && *name != "." && *name != "..")
            .ok_or(RecoveryError::ContainerInvalid)?;
        let name = CString::new(name.as_bytes()).map_err(|_| RecoveryError::ContainerInvalid)?;
        let parent_name = CString::new(parent_path.as_os_str().as_bytes())
            .map_err(|_| RecoveryError::PlatformUnsupported)?;
        let parent_fd = unsafe {
            libc::open(
                parent_name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if parent_fd < 0 {
            return Err(RecoveryError::Storage);
        }
        let parent = unsafe { File::from_raw_fd(parent_fd) };
        let dot = c".";
        let file_fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                dot.as_ptr(),
                libc::O_TMPFILE | libc::O_RDWR | libc::O_CLOEXEC,
                0o600,
            )
        };
        if file_fd < 0 {
            let error = io::Error::last_os_error();
            return if matches!(error.raw_os_error(), Some(code) if code == libc::EOPNOTSUPP
                || code == libc::ENOTSUP
                || code == libc::EINVAL)
            {
                Err(RecoveryError::PlatformUnsupported)
            } else {
                Err(RecoveryError::Storage)
            };
        }
        Ok(Self {
            parent,
            file: unsafe { File::from_raw_fd(file_fd) },
            name,
            ops,
        })
    }

    fn publish(self) -> Result<(), RecoveryError> {
        if let Err(error) = (self.ops.link)(&self.file, &self.parent, &self.name) {
            return if error.kind() == io::ErrorKind::AlreadyExists {
                Err(RecoveryError::DestinationExists)
            } else if matches!(error.raw_os_error(), Some(code) if code == libc::EOPNOTSUPP
                || code == libc::ENOTSUP
                || code == libc::EINVAL)
            {
                Err(RecoveryError::PlatformUnsupported)
            } else {
                Err(RecoveryError::Storage)
            };
        }
        if (self.ops.sync_directory)(&self.parent).is_err() {
            return Err(RecoveryError::ArtifactPublicationUncertain);
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
impl Write for LinuxEncryptedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        (self.ops.write)(&mut self.file, bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[cfg(target_os = "linux")]
impl EncryptionOutput for LinuxEncryptedOutput {
    fn sync_file(&mut self) -> io::Result<()> {
        (self.ops.sync_file)(&self.file)
    }
}

#[allow(dead_code)]
impl EncryptionOutput for File {
    fn sync_file(&mut self) -> io::Result<()> {
        self.sync_all()
    }
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
    consumer.finish_manifest()
}

/// Owns an anonymous plaintext staging file until the restore workflow consumes it.
///
/// The underlying file has no directory entry. Its owner is intentionally the
/// only way to access it; callers can read or seek through this type, but cannot
/// obtain a plaintext pathname or the underlying `File`.
pub struct DecryptedStage {
    file: File,
}

impl DecryptedStage {
    fn new(staging_directory: &Path) -> Result<Self, RecoveryError> {
        create_anonymous_plaintext_file(staging_directory).map(|file| Self { file })
    }

    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.file.write_all(bytes)
    }

    fn sync_all(&self) -> io::Result<()> {
        self.file.sync_all()
    }

    /// Rewinds the stage so the restore workflow can read it from byte zero.
    pub fn rewind(&mut self) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(0)).map(|_| ())
    }
}

#[cfg(target_os = "linux")]
fn create_anonymous_plaintext_file(staging_directory: &Path) -> Result<File, RecoveryError> {
    let directory = CString::new(staging_directory.as_os_str().as_bytes())
        .map_err(|_| RecoveryError::PlatformUnsupported)?;
    let directory_fd = unsafe {
        libc::open(
            directory.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if directory_fd < 0 {
        return Err(RecoveryError::Storage);
    }
    let directory = unsafe { File::from_raw_fd(directory_fd) };
    create_anonymous_plaintext_file_with(directory, linux_open_tmpfile)
}

#[cfg(target_os = "linux")]
fn create_anonymous_plaintext_file_with(
    directory: File,
    open_tmpfile: impl FnOnce(i32) -> io::Result<i32>,
) -> Result<File, RecoveryError> {
    let plaintext_fd = open_tmpfile(directory.as_raw_fd()).map_err(|error| {
        if matches!(error.raw_os_error(), Some(code) if code == libc::EOPNOTSUPP
            || code == libc::ENOTSUP
            || code == libc::EINVAL)
        {
            RecoveryError::PlatformUnsupported
        } else {
            RecoveryError::Storage
        }
    })?;
    Ok(unsafe { File::from_raw_fd(plaintext_fd) })
}

#[cfg(target_os = "linux")]
fn linux_open_tmpfile(directory_fd: i32) -> io::Result<i32> {
    let dot = c".";
    let plaintext_fd = unsafe {
        libc::openat(
            directory_fd,
            dot.as_ptr(),
            libc::O_TMPFILE | libc::O_RDWR | libc::O_CLOEXEC,
            0o600,
        )
    };
    if plaintext_fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(plaintext_fd)
    }
}

#[cfg(not(target_os = "linux"))]
fn create_anonymous_plaintext_file(_staging_directory: &Path) -> Result<File, RecoveryError> {
    Err(RecoveryError::PlatformUnsupported)
}

impl fmt::Debug for DecryptedStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DecryptedStage([REDACTED])")
    }
}

impl Read for DecryptedStage {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.file.read(bytes)
    }
}

impl Seek for DecryptedStage {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}

/// Authenticates and streams database plaintext into an anonymous staging file.
pub fn decrypt_container_to_staging_file(
    input: &Path,
    passphrase: &BackupPassphrase,
    staging_directory: &Path,
    plaintext_capacity_bytes: u64,
) -> Result<(DecryptedStage, NodeBackupManifest), RecoveryError> {
    validate_passphrase(passphrase)?;
    let mut file = File::open(input).map_err(|_| RecoveryError::Storage)?;
    let parsed = parse_header(&mut file)?;
    let key = derive_key(passphrase, &parsed.salt, &parsed.header)?;
    let cipher =
        XChaCha20Poly1305::new_from_slice(key.as_ref()).map_err(|_| RecoveryError::Cryptography)?;
    let mut consumer = PlaintextConsumer::new(Some(staging_directory), plaintext_capacity_bytes)?;
    consume_records(
        &mut file,
        &cipher,
        &parsed.nonce_prefix,
        &parsed.digest,
        parsed.header.chunk_size,
        &mut consumer,
    )?;
    consumer.finish_staging()
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
    if encoded.len() != 22 || !matches!(encoded.as_bytes().last(), Some(b'A' | b'Q' | b'g' | b'w'))
    {
        return Err(RecoveryError::ContainerInvalid);
    }
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

struct PlaintextConsumer<'a> {
    staging_directory: Option<&'a Path>,
    output_capacity: u64,
    staged: Option<DecryptedStage>,
    prefix: [u8; 4],
    prefix_len: usize,
    manifest_length: Option<usize>,
    manifest_bytes: Vec<u8>,
    manifest: Option<NodeBackupManifest>,
    database_length: u64,
    database_digest: Sha256,
}

impl<'a> PlaintextConsumer<'a> {
    fn new(
        staging_directory: Option<&'a Path>,
        output_capacity: u64,
    ) -> Result<Self, RecoveryError> {
        Ok(Self {
            staging_directory,
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
                    if let Some(directory) = self.staging_directory {
                        self.staged = Some(DecryptedStage::new(directory)?);
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

    fn finish_manifest(&mut self) -> Result<NodeBackupManifest, RecoveryError> {
        self.ensure_terminal()?;
        self.manifest.take().ok_or(RecoveryError::ManifestInvalid)
    }

    fn finish_staging(mut self) -> Result<(DecryptedStage, NodeBackupManifest), RecoveryError> {
        self.ensure_terminal()?;
        let staged = self.staged.take().ok_or(RecoveryError::ManifestInvalid)?;
        staged.sync_all().map_err(|_| RecoveryError::Storage)?;
        let manifest = self.manifest.take().ok_or(RecoveryError::ManifestInvalid)?;
        Ok((staged, manifest))
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

#[allow(dead_code)]
fn write_record(
    output: &mut impl Write,
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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
