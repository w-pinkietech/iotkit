use super::*;

#[test]
fn mqtt_packet_limit_covers_the_largest_custody_payload_and_topic() {
    let mut options = MqttOptions::new("test", "localhost", 1883);

    configure_packet_limits(&mut options);

    assert!(
        options.max_packet_size()
            >= MAX_BATCH_BYTES + MAX_MQTT_TOPIC_BYTES + MQTT_PACKET_OVERHEAD_BYTES
    );
}
