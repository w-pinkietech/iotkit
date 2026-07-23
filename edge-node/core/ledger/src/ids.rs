use crate::store::LedgerError;

/// 論理デバイスの主キー。UUIDv7・不変・台帳のみ発行・再利用永久禁止(D5決定1)。
/// DB内はBLOB16、API境界はTEXT36(D5決定3)。順序性には依存しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SystemId([u8; 16]);

impl SystemId {
    pub fn generate() -> Self {
        Self(*uuid::Uuid::now_v7().as_bytes())
    }
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
    pub fn from_bytes(b: [u8; 16]) -> Self {
        Self(b)
    }
    pub fn to_text(&self) -> String {
        uuid::Uuid::from_bytes(self.0).to_string()
    }
    pub fn from_text(s: &str) -> Result<Self, LedgerError> {
        uuid::Uuid::parse_str(s)
            .map(|u| Self(*u.as_bytes()))
            .map_err(|_| LedgerError::InvalidId(s.to_string()))
    }
}

#[cfg(test)]
#[path = "../tests/unit/ids_tests.rs"]
mod tests;
