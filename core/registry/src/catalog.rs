//! 標準語彙カタログ(D6決定1のリポジトリ資産層)。バイナリ同梱、起動時+ビルド時テストで整合検証。
use serde::Deserialize;
use std::sync::OnceLock;

pub const STANDARD_CATALOG_TOML: &str = include_str!("../catalog/standard-v1.toml");

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Catalog {
    pub catalog_version: String,
    #[serde(rename = "measurement")]
    pub measurements: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CatalogEntry {
    pub key: String,
    #[serde(default)]
    pub unit_ucum: Option<String>,
    #[serde(default)]
    pub unit_display: Option<String>,
    pub value_type: ValueType,
    pub semantic_class: String,
    pub channel_mode: ChannelMode,
    #[serde(default)]
    pub channel_roles: Vec<String>,
    #[serde(default)]
    pub physical_range: Option<Range>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueType {
    Float,
    Int,
    Bool,
    Record,
}

impl ValueType {
    pub fn as_db(&self) -> &'static str {
        match self {
            Self::Float => "float",
            Self::Int => "int",
            Self::Bool => "bool",
            Self::Record => "record",
        }
    }
    pub fn from_db(s: &str) -> Self {
        match s {
            "float" => Self::Float,
            "int" => Self::Int,
            "bool" => Self::Bool,
            "record" => Self::Record,
            other => {
                tracing::warn!(
                    value = other,
                    fallback = "float",
                    "unknown registry value_type in row"
                );
                Self::Float
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelMode {
    /// 単ch: channel_indexなし(またはSome(0))のみ正準
    Single,
    /// 汎用: デバイス側ラベルに委譲(D6決定12)。Wave 0は宣言照合なし=全channel_index受理
    Generic,
    /// 固定: カタログが役割を固定(index < roles.len() のみ正準)
    Fixed,
}

impl ChannelMode {
    pub fn as_db(&self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Generic => "generic",
            Self::Fixed => "fixed",
        }
    }
    pub fn from_db(s: &str) -> Self {
        match s {
            "single" => Self::Single,
            "generic" => Self::Generic,
            "fixed" => Self::Fixed,
            other => {
                tracing::warn!(
                    value = other,
                    fallback = "single",
                    "unknown registry channel_mode in row"
                );
                Self::Single
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Range {
    pub min: f64,
    pub max: f64,
}

#[derive(Debug)]
pub enum CatalogError {
    Parse(String),
    Invalid(String),
}
impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "catalog parse error: {e}"),
            Self::Invalid(e) => write!(f, "catalog invalid: {e}"),
        }
    }
}
impl std::error::Error for CatalogError {}

pub fn parse_catalog(toml_text: &str) -> Result<Catalog, CatalogError> {
    let catalog: Catalog =
        toml::from_str(toml_text).map_err(|e| CatalogError::Parse(e.to_string()))?;
    let mut seen = std::collections::HashSet::new();
    for m in &catalog.measurements {
        iotkit_ingest_contract::validate_measurement_key(&m.key)
            .map_err(|e| CatalogError::Invalid(format!("key '{}': {e}", m.key)))?;
        if !seen.insert(m.key.clone()) {
            return Err(CatalogError::Invalid(format!("duplicate key '{}'", m.key)));
        }
        match m.channel_mode {
            ChannelMode::Fixed if m.channel_roles.is_empty() => {
                return Err(CatalogError::Invalid(format!(
                    "key '{}': fixed channel_mode requires channel_roles",
                    m.key
                )));
            }
            ChannelMode::Single | ChannelMode::Generic if !m.channel_roles.is_empty() => {
                return Err(CatalogError::Invalid(format!(
                    "key '{}': channel_roles only allowed for fixed mode",
                    m.key
                )));
            }
            _ => {}
        }
        if let Some(r) = &m.physical_range
            && r.min.partial_cmp(&r.max) != Some(std::cmp::Ordering::Less)
        {
            return Err(CatalogError::Invalid(format!(
                "key '{}': physical_range min must be < max",
                m.key
            )));
        }
        if m.value_type == ValueType::Record && m.physical_range.is_some() {
            return Err(CatalogError::Invalid(format!(
                "key '{}': record type cannot carry a physical_range",
                m.key
            )));
        }
    }
    Ok(catalog)
}

/// 同梱カタログ(検証済み)。パース/検証失敗はプログラミングエラーなのでpanic
/// (ビルド時テストが同じ経路を通すため、壊れたカタログはCIで落ちる)。
pub fn standard_catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        parse_catalog(STANDARD_CATALOG_TOML).expect("bundled standard catalog must be valid")
    })
}

impl Catalog {
    pub fn find(&self, key: &str) -> Option<&CatalogEntry> {
        self.measurements.iter().find(|m| m.key == key)
    }
}

impl CatalogEntry {
    /// エントリrevision(内容ハッシュ、D6決定4)。カタログ版全体スタンプとは独立に
    /// 「このエントリの定義内容」を識別する。
    pub fn revision(&self) -> String {
        use sha2::{Digest, Sha256};
        let roles = self.channel_roles.join(",");
        let range = self
            .physical_range
            .map(|r| format!("{}..{}", r.min, r.max))
            .unwrap_or_default();
        let canonical = format!(
            "{}\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}\x1f{}",
            self.key,
            self.unit_ucum.as_deref().unwrap_or(""),
            self.unit_display.as_deref().unwrap_or(""),
            self.value_type.as_db(),
            self.semantic_class,
            self.channel_mode.as_db(),
            roles,
            range,
        );
        let hash = Sha256::digest(canonical.as_bytes());
        hash.iter().map(|b| format!("{b:02x}")).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_catalog_parses_with_10_keys_and_version() {
        let c = standard_catalog();
        assert_eq!(c.catalog_version, "1.0.0");
        assert_eq!(c.measurements.len(), 10);
        for k in [
            "contact_state",
            "contact_output_state",
            "voltage_mv",
            "distance_mm",
            "temperature_c",
            "acceleration_mg",
            "differential_pressure_pa",
            "illuminance_lux",
            "current_ma",
            "vibration_spectrum",
        ] {
            assert!(c.find(k).is_some(), "{k} must be in the standard catalog");
        }
    }

    #[test]
    fn acceleration_is_fixed_xyz_and_temperature_is_single() {
        let c = standard_catalog();
        let acc = c.find("acceleration_mg").unwrap();
        assert_eq!(acc.channel_mode, ChannelMode::Fixed);
        assert_eq!(acc.channel_roles, vec!["x", "y", "z"]);
        assert_eq!(
            acc.physical_range,
            Some(Range {
                min: -16000.0,
                max: 16000.0
            })
        );
        let t = c.find("temperature_c").unwrap();
        assert_eq!(t.channel_mode, ChannelMode::Single);
        assert_eq!(t.unit_ucum.as_deref(), Some("Cel"));
        assert_eq!(t.unit_display.as_deref(), Some("℃"));
    }

    #[test]
    fn vibration_spectrum_is_reserved_record() {
        let v = standard_catalog().find("vibration_spectrum").unwrap();
        assert_eq!(v.value_type, ValueType::Record);
        assert!(v.physical_range.is_none());
    }

    #[test]
    fn all_catalog_keys_pass_contract_grammar() {
        for m in &standard_catalog().measurements {
            assert!(
                iotkit_ingest_contract::validate_measurement_key(&m.key).is_ok(),
                "{} must satisfy D6決定2 grammar",
                m.key
            );
        }
    }

    #[test]
    fn parse_rejects_duplicate_keys() {
        let dup = r#"
catalog_version = "1.0.0"
[[measurement]]
key = "temperature_c"
value_type = "float"
semantic_class = "sensor"
channel_mode = "single"
[[measurement]]
key = "temperature_c"
value_type = "float"
semantic_class = "sensor"
channel_mode = "single"
"#;
        assert!(matches!(parse_catalog(dup), Err(CatalogError::Invalid(_))));
    }

    #[test]
    fn parse_rejects_fixed_without_roles_and_roles_on_non_fixed() {
        let fixed_no_roles = r#"
catalog_version = "1.0.0"
[[measurement]]
key = "a"
value_type = "float"
semantic_class = "sensor"
channel_mode = "fixed"
"#;
        assert!(matches!(
            parse_catalog(fixed_no_roles),
            Err(CatalogError::Invalid(_))
        ));
        let single_with_roles = r#"
catalog_version = "1.0.0"
[[measurement]]
key = "a"
value_type = "float"
semantic_class = "sensor"
channel_mode = "single"
channel_roles = ["x"]
"#;
        assert!(matches!(
            parse_catalog(single_with_roles),
            Err(CatalogError::Invalid(_))
        ));
    }

    #[test]
    fn parse_rejects_bad_key_grammar_and_inverted_range() {
        let bad_key = r#"
catalog_version = "1.0.0"
[[measurement]]
key = "Bad:Key"
value_type = "float"
semantic_class = "sensor"
channel_mode = "single"
"#;
        assert!(matches!(
            parse_catalog(bad_key),
            Err(CatalogError::Invalid(_))
        ));
        let inverted = r#"
catalog_version = "1.0.0"
[[measurement]]
key = "a"
value_type = "float"
semantic_class = "sensor"
channel_mode = "single"
physical_range = { min = 10.0, max = 1.0 }
"#;
        assert!(matches!(
            parse_catalog(inverted),
            Err(CatalogError::Invalid(_))
        ));
    }

    #[test]
    fn revision_is_stable_and_content_sensitive() {
        let c = standard_catalog();
        let t = c.find("temperature_c").unwrap();
        let r1 = t.revision();
        let r2 = t.revision();
        assert_eq!(r1, r2, "same content → same revision");
        assert_eq!(r1.len(), 64, "sha256 hex");
        let mut altered = t.clone();
        altered.physical_range = Some(Range {
            min: -200.0,
            max: 9999.0,
        });
        assert_ne!(r1, altered.revision(), "content change → revision change");
    }
}
