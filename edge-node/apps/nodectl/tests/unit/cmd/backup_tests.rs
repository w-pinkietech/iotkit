#[cfg(target_os = "linux")]
#[test]
fn requires_mounts_for_path_uses_systemd_hex_escapes_for_special_bytes() {
    let path = std::path::Path::new("/mnt/a b\\c\"%/制御");
    assert_eq!(
        super::systemd_mount_path(path).unwrap(),
        "/mnt/a\\x20b\\x5cc\\x22\\x25/\\xe5\\x88\\xb6\\xe5\\xbe\\xa1"
    );
    let drop_in = String::from_utf8(super::systemd_drop_in_bytes(path).unwrap()).unwrap();
    assert_eq!(
        drop_in,
        "[Unit]\nRequiresMountsFor=/mnt/a\\x20b\\x5cc\\x22\\x25/\\xe5\\x88\\xb6\\xe5\\xbe\\xa1\n"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn requires_mounts_for_path_rejects_non_utf8_input() {
    use std::os::unix::ffi::OsStringExt;

    let path = std::path::PathBuf::from(std::ffi::OsString::from_vec(vec![
        b'/', b'm', b'n', b't', b'/', 0xff,
    ]));
    assert!(matches!(
        super::systemd_mount_path(&path),
        Err(iotkit_core_recovery::RecoveryError::InvalidConfiguration)
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn restore_staging_is_created_owner_only_even_with_umask_zero() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let _environment_lock = environment_lock().lock().unwrap();

    let old_umask = unsafe { libc::umask(0) };
    let path = super::create_restore_staging().unwrap();
    unsafe { libc::umask(old_umask) };
    let metadata = std::fs::metadata(&path).unwrap();
    assert!(metadata.is_dir());
    assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
    assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
    std::fs::remove_dir(path).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn restore_staging_post_create_failure_removes_private_directory() {
    let _environment_lock = environment_lock().lock().unwrap();
    let base = tempfile::tempdir_in("/tmp").unwrap();
    let previous_tmpdir = std::env::var_os("TMPDIR");
    unsafe {
        std::env::set_var("TMPDIR", base.path());
        std::env::set_var("IOTKIT_TEST_RESTORE_STAGING_FAIL_AFTER_CREATE", "1");
    }
    let result = super::create_restore_staging();
    unsafe {
        std::env::remove_var("IOTKIT_TEST_RESTORE_STAGING_FAIL_AFTER_CREATE");
    }
    match previous_tmpdir {
        Some(value) => unsafe { std::env::set_var("TMPDIR", value) },
        None => unsafe { std::env::remove_var("TMPDIR") },
    }
    assert!(matches!(
        result,
        Err(iotkit_core_recovery::RecoveryError::Storage)
    ));
    assert_eq!(std::fs::read_dir(base.path()).unwrap().count(), 0);
}

#[cfg(target_os = "linux")]
fn environment_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}
