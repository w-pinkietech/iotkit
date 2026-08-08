use iotkit_edge_custody_contract::{
    AcceptedThrough, ActivationResult, DescriptorSnapshot, RecordBatch, RecoveryActivationResult,
    RecoveryCompletionAck, SCHEMA_VERSION,
};

use crate::storage::{AcceptBatch, RawRecord, Storage, StorageError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AckPublication {
    pub topic: String,
    pub retain: bool,
    pub payload: Vec<u8>,
}

#[derive(Clone)]
pub struct IngestProcessor {
    storage: Storage,
}

impl IngestProcessor {
    #[must_use]
    pub fn new(storage: Storage) -> Self {
        Self { storage }
    }

    pub(crate) fn storage(&self) -> Storage {
        self.storage.clone()
    }

    pub async fn handle(
        &self,
        topic: &str,
        payload: &[u8],
        received_at: i64,
    ) -> Result<Option<AckPublication>, IngestError> {
        let parsed = Topic::parse(topic)?;
        match parsed.kind {
            TopicKind::Descriptor => {
                let descriptor = DescriptorSnapshot::decode(payload)?;
                descriptor.validate_topic_edge_node(&parsed.edge_node_id)?;
                self.storage
                    .apply_descriptor(&descriptor, received_at)
                    .await?;
                Ok(None)
            }
            TopicKind::ActivationResult => {
                let result = ActivationResult::decode(payload)?;
                result.validate_topic_edge_node(&parsed.edge_node_id)?;
                self.storage
                    .apply_activation_result(&result, received_at)
                    .await?;
                Ok(None)
            }
            TopicKind::RecoveryResult => {
                let result = RecoveryActivationResult::decode(payload)?;
                result.validate_topic_edge_node(&parsed.edge_node_id)?;
                self.storage
                    .apply_edge_node_recovery_result(&result, received_at)
                    .await?;
                Ok(None)
            }
            TopicKind::RecoveryCompletionAck => {
                let acknowledgement = RecoveryCompletionAck::decode(payload)?;
                acknowledgement.validate_topic_edge_node(&parsed.edge_node_id)?;
                self.storage
                    .acknowledge_edge_node_recovery_completion(&acknowledgement, received_at)
                    .await?;
                Ok(None)
            }
            TopicKind::Records => {
                let batch = RecordBatch::decode(payload)?;
                batch.validate_topic_edge_node(&parsed.edge_node_id)?;
                self.storage
                    .accept_active_batch(AcceptBatch {
                        edge_node_id: batch.edge_node_id.clone(),
                        ledger_epoch: batch.ledger_epoch.clone(),
                        publication_id: batch.publication_id.clone(),
                        received_at,
                        records: batch
                            .records
                            .iter()
                            .enumerate()
                            .map(|(index, record)| {
                                RawRecord::new(batch.cursor_start + index as i64, record.get())
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                    })
                    .await?;
                let ack = AcceptedThrough {
                    schema_version: SCHEMA_VERSION,
                    edge_node_id: batch.edge_node_id.clone(),
                    ledger_epoch: batch.ledger_epoch.clone(),
                    publication_id: batch.publication_id.clone(),
                    accepted_through: batch.cursor_end,
                };
                ack.validate_for(&batch, batch.cursor_start - 1)?;
                Ok(Some(AckPublication {
                    topic: format!(
                        "iotkit/v1/edge-nodes/{}/accepted-through",
                        batch.edge_node_id
                    ),
                    retain: false,
                    payload: serde_json::to_vec(&ack)?,
                }))
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("invalid MQTT topic: {0}")]
    Topic(String),
    #[error("invalid MQTT contract: {0}")]
    Contract(#[from] iotkit_edge_custody_contract::ContractError),
    #[error("MQTT custody storage failed: {0}")]
    Storage(#[from] StorageError),
    #[error("encode MQTT acknowledgement: {0}")]
    Encode(#[from] serde_json::Error),
}

impl IngestError {
    #[must_use]
    pub fn is_fatal_runtime(&self) -> bool {
        matches!(self, Self::Storage(StorageError::Database(_)))
    }
}

enum TopicKind {
    Descriptor,
    ActivationResult,
    RecoveryResult,
    RecoveryCompletionAck,
    Records,
}

struct Topic {
    edge_node_id: String,
    kind: TopicKind,
}

impl Topic {
    fn parse(topic: &str) -> Result<Self, IngestError> {
        let parts: Vec<&str> = topic.split('/').collect();
        let (edge_node_id, kind) = match parts.as_slice() {
            ["iotkit", "v1", "edge-nodes", edge_node_id, "descriptors"] => {
                (*edge_node_id, TopicKind::Descriptor)
            }
            ["iotkit", "v1", "edge-nodes", edge_node_id, "records"] => {
                (*edge_node_id, TopicKind::Records)
            }
            [
                "iotkit",
                "v1",
                "edge-nodes",
                edge_node_id,
                "activation",
                "result",
            ] => (*edge_node_id, TopicKind::ActivationResult),
            [
                "iotkit",
                "v1",
                "edge-nodes",
                edge_node_id,
                "recovery",
                "result",
            ] => (*edge_node_id, TopicKind::RecoveryResult),
            [
                "iotkit",
                "v1",
                "edge-nodes",
                edge_node_id,
                "recovery",
                "completion-ack",
            ] => (*edge_node_id, TopicKind::RecoveryCompletionAck),
            _ => return Err(IngestError::Topic("unexpected topic shape".into())),
        };
        if edge_node_id.is_empty() || edge_node_id.contains(['+', '#', ':']) {
            return Err(IngestError::Topic("unsafe Edge Node identity".into()));
        }
        Ok(Self {
            edge_node_id: edge_node_id.into(),
            kind,
        })
    }
}
