use rand::Rng;

/// Generate a 32-character lowercase hex session ID using cryptographic randomness.
pub fn generate_session_id() -> String {
    let mut rng = rand::thread_rng();
    let bytes: [u8; 16] = rng.r#gen();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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
        let id2 = generate_session_id();
        assert_ne!(id1, id2, "consecutive session_ids must differ");
    }
}
