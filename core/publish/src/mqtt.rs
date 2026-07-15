use serde::Serialize;

use crate::wire::WireError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MqttBinding {
    pub edge_node_id: String,
    pub username: String,
    pub client_id: String,
    pub records_topic: String,
    pub accepted_through_topic: String,
    pub qos: u8,
    pub retain: bool,
}

impl MqttBinding {
    pub fn for_edge(edge_node_id: &str) -> Result<Self, WireError> {
        crate::wire::validate_topic_segment("edge_node_id", edge_node_id)?;
        Ok(Self {
            edge_node_id: edge_node_id.to_string(),
            username: edge_node_id.to_string(),
            client_id: format!("iotkit-edge-{edge_node_id}"),
            records_topic: format!("iotkit/v1/edge-nodes/{edge_node_id}/records"),
            accepted_through_topic: format!("iotkit/v1/edge-nodes/{edge_node_id}/accepted-through"),
            qos: 1,
            retain: false,
        })
    }
}
