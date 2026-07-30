use iotkit_core_publish::mqtt::MqttBinding;

#[test]
fn mqtt_binding_derives_the_d9_identity_and_topics() {
    let binding = MqttBinding::for_edge_node("edge-node-01").unwrap();

    assert_eq!(binding.edge_node_id, "edge-node-01");
    assert_eq!(binding.username, "edge-node-01");
    assert_eq!(binding.client_id, "iotkit-edge-node-edge-node-01");
    assert_eq!(
        binding.records_topic,
        "iotkit/v1/edge-nodes/edge-node-01/records"
    );
    assert_eq!(
        binding.accepted_through_topic,
        "iotkit/v1/edge-nodes/edge-node-01/accepted-through"
    );
    assert_eq!(
        binding.descriptor_topic,
        "iotkit/v1/edge-nodes/edge-node-01/descriptors"
    );
    assert_eq!(
        binding.activation_request_topic,
        "iotkit/v1/edge-nodes/edge-node-01/activation/request"
    );
    assert_eq!(
        binding.activation_result_topic,
        "iotkit/v1/edge-nodes/edge-node-01/activation/result"
    );
    assert_eq!(
        binding.recovery_request_topic,
        "iotkit/v1/edge-nodes/edge-node-01/recovery/request"
    );
    assert_eq!(
        binding.recovery_result_topic,
        "iotkit/v1/edge-nodes/edge-node-01/recovery/result"
    );
    assert_eq!(
        binding.recovery_completion_topic,
        "iotkit/v1/edge-nodes/edge-node-01/recovery/completion"
    );
    assert_eq!(
        binding.recovery_completion_ack_topic,
        "iotkit/v1/edge-nodes/edge-node-01/recovery/completion-ack"
    );
    assert_eq!(binding.qos, 1);
    assert!(!binding.retain);
    assert!(binding.descriptor_retain);
}

#[test]
fn mqtt_binding_rejects_an_unsafe_edge_node_id() {
    for edge_node_id in ["", "edge/node", "edge+node", "edge#node", "edge:node"] {
        assert!(
            MqttBinding::for_edge_node(edge_node_id).is_err(),
            "{edge_node_id:?}"
        );
    }
}
