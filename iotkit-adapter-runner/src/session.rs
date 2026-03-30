use std::time::{SystemTime, UNIX_EPOCH};

/// Generate a 32-character lowercase hex session ID.
/// Unique per process lifetime. Uses nanosecond timestamp + PID scramble.
pub fn generate_session_id() -> String {
    let high = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let low = std::process::id() as u64 ^ high.wrapping_mul(0x517cc1b727220a95);
    format!("{high:016x}{low:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_is_32_hex_chars() {
        let id = generate_session_id();
        assert_eq!(id.len(), 32, "session_id must be 32 chars, got {}", id.len());
        assert!(
            id.chars().all(|c| c.is_ascii_hexdigit()),
            "session_id must be hex, got {id}"
        );
        assert_eq!(id, id.to_lowercase(), "session_id must be lowercase");
    }

    #[test]
    fn session_ids_are_unique() {
        let id1 = generate_session_id();
        std::thread::sleep(std::time::Duration::from_millis(1));
        let id2 = generate_session_id();
        assert_ne!(id1, id2, "consecutive session_ids must differ");
    }
}
