use crate::{MqttConfig, RunnerError};
use iotkit_core_mqtt_contract::{encode_status, topic, EventType};
use iotkit_core_types::AdapterId;
use rumqttc::{AsyncClient, EventLoop, MqttOptions, QoS, Transport};
use std::time::Duration;
use url::Url;

/// Create and configure an MQTT client with LWT.
pub(crate) fn connect(
    adapter_id: &AdapterId,
    config: &MqttConfig,
) -> Result<(AsyncClient, EventLoop), RunnerError> {
    let client_id = config.client_id.clone().unwrap_or_else(|| {
        format!(
            "iotkit-{}-{}",
            adapter_id.as_str().replace(':', "-"),
            &uuid_short()
        )
    });

    let keepalive = Duration::from_secs(config.keepalive_secs.unwrap_or(30) as u64);

    let (host, port) = parse_broker_url(&config.broker_url)?;
    let mut opts = MqttOptions::new(&client_id, host, port);
    opts.set_keep_alive(keepalive);

    // Validate mTLS: both cert and key must be provided, or neither
    if config.client_cert_path.is_some() != config.client_key_path.is_some() {
        return Err(RunnerError::Config(
            "both client_cert_path and client_key_path must be set for mTLS, or neither".into(),
        ));
    }

    // TLS configuration
    if config.broker_url.starts_with("mqtts://") {
        let ca = config
            .ca_path
            .as_ref()
            .ok_or_else(|| RunnerError::Config("mqtts:// requires ca_path".into()))?;
        let ca_bytes = std::fs::read(ca)
            .map_err(|e| RunnerError::Config(format!("failed to read CA cert: {e}")))?;

        let transport = if let (Some(cert_path), Some(key_path)) =
            (&config.client_cert_path, &config.client_key_path)
        {
            let cert = std::fs::read(cert_path)
                .map_err(|e| RunnerError::Config(format!("failed to read client cert: {e}")))?;
            let key = std::fs::read(key_path)
                .map_err(|e| RunnerError::Config(format!("failed to read client key: {e}")))?;
            rumqttc::TlsConfiguration::Simple {
                ca: ca_bytes,
                alpn: None,
                client_auth: Some((cert, key)),
            }
        } else {
            rumqttc::TlsConfiguration::Simple {
                ca: ca_bytes,
                alpn: None,
                client_auth: None,
            }
        };

        opts.set_transport(Transport::tls_with_config(transport.into()));
    }

    // Last Will and Testament - offline status with ts=0 (LWT time unknown)
    let lwt_topic = topic(adapter_id, EventType::Status);
    let lwt_payload = encode_status(adapter_id, false, 0);
    opts.set_last_will(rumqttc::LastWill::new(
        &lwt_topic,
        lwt_payload,
        QoS::AtLeastOnce,
        true, // retained
    ));

    let (client, eventloop) = AsyncClient::new(opts, 100);
    Ok((client, eventloop))
}

fn parse_broker_url(broker_url: &str) -> Result<(String, u16), RunnerError> {
    // Replace mqtt:// with http:// for url crate compatibility
    let normalized = broker_url
        .replacen("mqtt://", "http://", 1)
        .replacen("mqtts://", "https://", 1);

    if normalized == broker_url {
        return Err(RunnerError::Config(
            "broker_url must start with mqtt:// or mqtts://".into(),
        ));
    }

    let parsed = Url::parse(&normalized)
        .map_err(|e| RunnerError::Config(format!("invalid broker_url: {e}")))?;

    let host = parsed
        .host_str()
        .ok_or_else(|| RunnerError::Config("broker_url has no host".into()))?
        .to_string();

    let default_port = if broker_url.starts_with("mqtts://") {
        8883
    } else {
        1883
    };
    let port = parsed.port().unwrap_or(default_port);

    Ok((host, port))
}

fn uuid_short() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:08x}", (n & 0xFFFF_FFFF) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mqtt_url() {
        let (host, port) = parse_broker_url("mqtt://localhost:1883").unwrap();
        assert_eq!(host, "localhost");
        assert_eq!(port, 1883);
    }

    #[test]
    fn parse_mqtts_url_default_port() {
        let (host, port) = parse_broker_url("mqtts://broker.example.com").unwrap();
        assert_eq!(host, "broker.example.com");
        assert_eq!(port, 8883);
    }

    #[test]
    fn parse_invalid_url() {
        assert!(parse_broker_url("http://localhost").is_err());
    }

    #[test]
    fn parse_ipv6_url() {
        let (host, port) = parse_broker_url("mqtt://[fd00::1]:1883").unwrap();
        assert_eq!(host, "[fd00::1]");
        assert_eq!(port, 1883);
    }

    #[test]
    fn parse_mqtt_default_port() {
        let (host, port) = parse_broker_url("mqtt://localhost").unwrap();
        assert_eq!(host, "localhost");
        assert_eq!(port, 1883);
    }

    #[test]
    fn half_configured_mtls_cert_only_rejected() {
        let config = MqttConfig {
            broker_url: "mqtt://localhost:1883".into(),
            client_id: None,
            keepalive_secs: None,
            ca_path: None,
            client_cert_path: Some("/tmp/cert.pem".into()),
            client_key_path: None,
        };
        let adapter_id = iotkit_core_types::AdapterId::new("test");
        match connect(&adapter_id, &config) {
            Err(e) => {
                let msg = e.to_string();
                assert!(msg.contains("client_cert_path"), "error = {msg}");
                assert!(msg.contains("client_key_path"), "error = {msg}");
            }
            Ok(_) => panic!("expected error for half-configured mTLS"),
        }
    }

    #[test]
    fn half_configured_mtls_key_only_rejected() {
        let config = MqttConfig {
            broker_url: "mqtt://localhost:1883".into(),
            client_id: None,
            keepalive_secs: None,
            ca_path: None,
            client_cert_path: None,
            client_key_path: Some("/tmp/key.pem".into()),
        };
        let adapter_id = iotkit_core_types::AdapterId::new("test");
        match connect(&adapter_id, &config) {
            Err(e) => {
                let msg = e.to_string();
                assert!(msg.contains("client_cert_path"), "error = {msg}");
            }
            Ok(_) => panic!("expected error for half-configured mTLS"),
        }
    }
}
