use std::{fmt, fs, net::SocketAddr};

use url::Url;

use crate::{
    cli::{ServeArgs, read_owner_only_secret},
    storage::StorageProfile,
};

#[derive(Clone)]
pub enum MqttTransportConfig {
    TlsSystemRoots,
    TlsBundle { ca_pem: Vec<u8> },
    PlaintextForDevelopment,
}

impl fmt::Debug for MqttTransportConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TlsSystemRoots => formatter.write_str("TlsSystemRoots"),
            Self::TlsBundle { .. } => formatter.write_str("TlsBundle([REDACTED])"),
            Self::PlaintextForDevelopment => formatter.write_str("PlaintextForDevelopment"),
        }
    }
}

#[derive(Clone)]
pub struct MqttConnectionConfig {
    pub host: String,
    pub port: u16,
    pub client_id: String,
    pub username: String,
    pub password: String,
    pub transport: MqttTransportConfig,
}

impl fmt::Debug for MqttConnectionConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MqttConnectionConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("client_id", &self.client_id)
            .field("username", &"[REDACTED]")
            .field("password", &"[REDACTED]")
            .field("transport", &self.transport)
            .finish()
    }
}

pub struct RuntimeConfig {
    pub storage: StorageProfile,
    pub edge_id: String,
    pub ingest: MqttConnectionConfig,
    pub output: Option<MqttConnectionConfig>,
    pub http_listen: SocketAddr,
    pub public_origin: String,
    pub secure_cookies: bool,
}

impl fmt::Debug for RuntimeConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeConfig")
            .field("storage", &self.storage)
            .field("edge_id", &self.edge_id)
            .field("ingest", &self.ingest)
            .field("output", &self.output)
            .field("http_listen", &self.http_listen)
            .field("public_origin", &self.public_origin)
            .field("secure_cookies", &self.secure_cookies)
            .finish()
    }
}

impl RuntimeConfig {
    pub fn from_serve_args(args: &ServeArgs) -> Result<Self, RuntimeConfigError> {
        let storage = args
            .storage
            .storage_profile()
            .map_err(|error| RuntimeConfigError::Storage(error.to_string()))?;
        if !valid_edge_id(&args.edge_id) {
            return Err(RuntimeConfigError::Invalid(
                "edge ID must be edge- followed by 32 lowercase hexadecimal characters",
            ));
        }
        let ingest = mqtt_connection(
            &args.broker_url,
            &args.client_id,
            &args.username,
            &args.password_file,
            args.trust_mode.as_deref(),
            args.ca_file.as_deref(),
            args.allow_insecure,
        )?;
        let has_output_detail = args.output_username.is_some()
            || args.output_password_file.is_some()
            || args.output_trust_mode.is_some()
            || args.output_ca_file.is_some()
            || args.output_allow_insecure;
        if args.output_broker_url.is_none() && has_output_detail {
            return Err(RuntimeConfigError::Invalid(
                "output settings require output broker URL",
            ));
        }
        let output = args
            .output_broker_url
            .as_deref()
            .map(|broker_url| {
                let username = args
                    .output_username
                    .as_deref()
                    .ok_or(RuntimeConfigError::Invalid("output username is required"))?;
                let password_file =
                    args.output_password_file
                        .as_deref()
                        .ok_or(RuntimeConfigError::Invalid(
                            "output password file is required",
                        ))?;
                mqtt_connection(
                    broker_url,
                    &args.output_client_id,
                    username,
                    password_file,
                    args.output_trust_mode.as_deref(),
                    args.output_ca_file.as_deref(),
                    args.output_allow_insecure,
                )
            })
            .transpose()?;
        let http_listen = args
            .http_listen
            .parse()
            .map_err(|_| RuntimeConfigError::Invalid("invalid HTTP listen address"))?;
        let origin = Url::parse(&args.public_origin)
            .map_err(|_| RuntimeConfigError::Invalid("invalid public origin"))?;
        if origin.origin().ascii_serialization() != args.public_origin
            || (!args.development_http && origin.scheme() != "https")
            || (args.development_http && !matches!(origin.scheme(), "http" | "https"))
        {
            return Err(RuntimeConfigError::Invalid("invalid public origin"));
        }
        Ok(Self {
            storage,
            edge_id: args.edge_id.clone(),
            ingest,
            output,
            http_listen,
            public_origin: args.public_origin.clone(),
            secure_cookies: !args.development_http,
        })
    }
}

fn mqtt_connection(
    broker_url: &str,
    client_id: &str,
    username: &str,
    password_file: &std::path::Path,
    trust_mode: Option<&str>,
    ca_file: Option<&std::path::Path>,
    allow_insecure: bool,
) -> Result<MqttConnectionConfig, RuntimeConfigError> {
    if client_id.is_empty() || username.is_empty() {
        return Err(RuntimeConfigError::Invalid(
            "MQTT client ID and username are required",
        ));
    }
    let endpoint =
        Url::parse(broker_url).map_err(|_| RuntimeConfigError::Invalid("invalid broker URL"))?;
    if !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || !matches!(endpoint.path(), "" | "/")
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(RuntimeConfigError::Invalid("invalid broker URL"));
    }
    let host = endpoint
        .host_str()
        .filter(|host| !host.is_empty())
        .ok_or(RuntimeConfigError::Invalid("broker host is required"))?;
    let port = endpoint
        .port()
        .ok_or(RuntimeConfigError::Invalid("broker port is required"))?;
    let transport = match (endpoint.scheme(), allow_insecure, trust_mode, ca_file) {
        ("tcp", true, None, None) => MqttTransportConfig::PlaintextForDevelopment,
        ("ssl", false, Some("system_roots"), None) => MqttTransportConfig::TlsSystemRoots,
        ("ssl", false, Some("bundle_only"), Some(path)) => {
            let ca_pem = fs::read(path)?;
            if ca_pem.is_empty() {
                return Err(RuntimeConfigError::Invalid("CA bundle is empty"));
            }
            MqttTransportConfig::TlsBundle { ca_pem }
        }
        _ => {
            return Err(RuntimeConfigError::Invalid(
                "broker scheme, trust mode, CA file, and insecure flag conflict",
            ));
        }
    };
    let password = read_owner_only_secret(password_file)
        .map_err(|error| RuntimeConfigError::Secret(error.to_string()))?;
    Ok(MqttConnectionConfig {
        host: host.into(),
        port,
        client_id: client_id.into(),
        username: username.into(),
        password,
        transport,
    })
}

fn valid_edge_id(value: &str) -> bool {
    let suffix = value.strip_prefix("edge-").unwrap_or_default();
    suffix.len() == 32
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeConfigError {
    #[error("invalid runtime configuration: {0}")]
    Invalid(&'static str),
    #[error("storage configuration failed: {0}")]
    Storage(String),
    #[error("secret input failed: {0}")]
    Secret(String),
    #[error("read runtime file: {0}")]
    Io(#[from] std::io::Error),
}
