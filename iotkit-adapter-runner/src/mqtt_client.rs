use crate::{MqttConfig, RunnerError};
use iotkit_core_mqtt_contract::{encode_status, EventType};
use iotkit_core_types::AdapterId;
use rumqttc::{AsyncClient, EventLoop, LastWill, MqttOptions, QoS};
use std::time::Duration;

/// Parse broker_url into (host, port, tls).
pub(crate) fn parse_broker_url(raw: &str) -> Result<(String, u16, bool), RunnerError> {
    let (substituted, default_port, tls) = if let Some(rest) = raw.strip_prefix("mqtts://") {
        (format!("https://{rest}"), 8883u16, true)
    } else if let Some(rest) = raw.strip_prefix("mqtt://") {
        (format!("http://{rest}"), 1883u16, false)
    } else {
        return Err(RunnerError::MqttInit(format!(
            "broker_url scheme must be mqtt:// or mqtts://, got: {raw}"
        )));
    };

    let parsed = url::Url::parse(&substituted)
        .map_err(|e| RunnerError::MqttInit(format!("invalid broker_url: {e}")))?;

    let host = parsed
        .host_str()
        .ok_or_else(|| RunnerError::MqttInit("broker_url has no host".to_string()))?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();

    let port = parsed.port().unwrap_or(default_port);

    Ok((host, port, tls))
}

/// Build the LWT (Last Will and Testament) payload.
pub(crate) fn build_lwt(adapter_id: &AdapterId, session_id: &str) -> LastWill {
    let topic = iotkit_core_mqtt_contract::topic(adapter_id, EventType::Status);
    let payload = encode_status(adapter_id, false, 0, session_id);
    LastWill {
        topic,
        message: payload.into(),
        qos: QoS::AtLeastOnce,
        retain: true,
    }
}

/// Create the MQTT client and EventLoop.
///
/// Returns `(AsyncClient, EventLoop)`.
/// The caller spawns the eventloop task.
pub(crate) fn create_mqtt_client(
    adapter_id: &AdapterId,
    config: &MqttConfig,
    session_id: &str,
) -> Result<(AsyncClient, EventLoop), RunnerError> {
    let (host, port, tls) = parse_broker_url(&config.broker_url)?;

    let client_id = config.client_id.clone().unwrap_or_else(|| {
        format!(
            "iotkit-{}",
            percent_encoding::utf8_percent_encode(
                adapter_id.as_str(),
                percent_encoding::NON_ALPHANUMERIC,
            )
        )
    });

    let keepalive = config.keepalive_secs.unwrap_or(30);
    let mut opts = MqttOptions::new(&client_id, &host, port);
    opts.set_keep_alive(Duration::from_secs(keepalive as u64));
    opts.set_clean_session(true);
    opts.set_last_will(build_lwt(adapter_id, session_id));

    // TLS configuration for mqtts://
    if tls {
        use rumqttc::TlsConfiguration;
        use std::fs;

        let ca = fs::read(config.ca_path.as_ref().ok_or_else(|| {
            RunnerError::MqttInit("ca_path required for mqtts://".into())
        })?)
        .map_err(|e| RunnerError::MqttInit(format!("failed to read ca_path: {e}")))?;

        let client_auth = match (&config.client_cert_path, &config.client_key_path) {
            (Some(cert), Some(key)) => {
                let cert_bytes = fs::read(cert).map_err(|e| {
                    RunnerError::MqttInit(format!("failed to read client_cert_path: {e}"))
                })?;
                let key_bytes = fs::read(key).map_err(|e| {
                    RunnerError::MqttInit(format!("failed to read client_key_path: {e}"))
                })?;
                Some((cert_bytes, key_bytes))
            }
            _ => None,
        };

        let tls_config = if let Some((cert, key)) = client_auth {
            TlsConfiguration::Simple {
                ca: ca.into(),
                alpn: None,
                client_auth: Some((cert.into(), key.into())),
            }
        } else {
            TlsConfiguration::Simple {
                ca: ca.into(),
                alpn: None,
                client_auth: None,
            }
        };

        opts.set_transport(rumqttc::Transport::tls_with_config(tls_config.into()));
    }

    // Extract username/password from URL if present
    {
        let substituted = if config.broker_url.starts_with("mqtts://") {
            format!("https://{}", &config.broker_url[8..])
        } else {
            format!("http://{}", &config.broker_url[7..])
        };
        if let Ok(parsed) = url::Url::parse(&substituted) {
            let username = parsed.username();
            let password = parsed.password();
            if !username.is_empty() {
                opts.set_credentials(username, password.unwrap_or(""));
            }
        }
    }

    let (client, eventloop) = AsyncClient::new(opts, 100);
    Ok((client, eventloop))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mqtt_url_default_port() {
        let (host, port, tls) = parse_broker_url("mqtt://localhost").unwrap();
        assert_eq!(host, "localhost");
        assert_eq!(port, 1883);
        assert!(!tls);
    }

    #[test]
    fn parse_mqtts_url_default_port() {
        let (host, port, tls) = parse_broker_url("mqtts://broker.example.com").unwrap();
        assert_eq!(host, "broker.example.com");
        assert_eq!(port, 8883);
        assert!(tls);
    }

    #[test]
    fn parse_mqtt_url_custom_port() {
        let (host, port, _) = parse_broker_url("mqtt://10.0.0.1:9883").unwrap();
        assert_eq!(host, "10.0.0.1");
        assert_eq!(port, 9883);
    }

    #[test]
    fn parse_mqtt_url_ipv6() {
        let (host, port, _) = parse_broker_url("mqtt://[::1]:1883").unwrap();
        assert!(!host.starts_with('['), "brackets must be stripped: {host}");
        assert_eq!(port, 1883);
    }

    #[test]
    fn parse_invalid_scheme() {
        assert!(parse_broker_url("tcp://localhost").is_err());
    }

    #[test]
    fn lwt_payload_has_ts_zero_and_session_id() {
        let adapter_id = AdapterId::new("test:adapter");
        let session_id = "abcd1234abcd1234abcd1234abcd1234";
        let lwt = build_lwt(&adapter_id, session_id);
        let json: serde_json::Value = serde_json::from_slice(&lwt.message).unwrap();
        assert_eq!(json["ts"], 0);
        assert_eq!(json["online"], false);
        assert_eq!(json["session_id"], session_id);
    }
}
