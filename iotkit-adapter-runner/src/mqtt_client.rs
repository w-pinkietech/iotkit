use crate::{MqttConfig, RunnerError};
use iotkit_core_mqtt_contract::{encode_status, topic, EventType};
use iotkit_core_types::AdapterId;
use rumqttc::{AsyncClient, EventLoop, MqttOptions, QoS, Transport};
use std::time::Duration;

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

    let mut opts = MqttOptions::new(
        &client_id,
        parse_host(&config.broker_url)?,
        parse_port(&config.broker_url)?,
    );
    opts.set_keep_alive(keepalive);

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

fn parse_host(url: &str) -> Result<String, RunnerError> {
    let stripped = url
        .strip_prefix("mqtt://")
        .or_else(|| url.strip_prefix("mqtts://"))
        .ok_or_else(|| {
            RunnerError::Config("broker_url must start with mqtt:// or mqtts://".into())
        })?;
    let host = stripped.split(':').next().unwrap_or(stripped);
    Ok(host.to_string())
}

fn parse_port(url: &str) -> Result<u16, RunnerError> {
    let stripped = url
        .strip_prefix("mqtt://")
        .or_else(|| url.strip_prefix("mqtts://"))
        .ok_or_else(|| {
            RunnerError::Config("broker_url must start with mqtt:// or mqtts://".into())
        })?;
    let parts: Vec<&str> = stripped.split(':').collect();
    if parts.len() >= 2 {
        parts[1]
            .parse()
            .map_err(|_| RunnerError::Config(format!("invalid port in broker_url: {}", parts[1])))
    } else if url.starts_with("mqtts://") {
        Ok(8883)
    } else {
        Ok(1883)
    }
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
        assert_eq!(parse_host("mqtt://localhost:1883").unwrap(), "localhost");
        assert_eq!(parse_port("mqtt://localhost:1883").unwrap(), 1883);
    }

    #[test]
    fn parse_mqtts_url_default_port() {
        assert_eq!(
            parse_host("mqtts://broker.example.com").unwrap(),
            "broker.example.com"
        );
        assert_eq!(parse_port("mqtts://broker.example.com").unwrap(), 8883);
    }

    #[test]
    fn parse_invalid_url() {
        assert!(parse_host("http://localhost").is_err());
    }
}
