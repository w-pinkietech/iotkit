fn validate_rule_draft(draft: &SemanticRuleDraft, now: i64) -> Result<(), StorageError> {
    if draft.edge_node_id.is_empty()
        || draft.series_key.is_empty()
        || draft.display_name.is_empty()
        || draft.display_name.len() > 128
        || now < 0
    {
        return Err(StorageError::InvalidSemantic(
            "rule identity, display name, and timestamp are required".into(),
        ));
    }
    Ok(())
}

fn semantic_kind(value: SemanticKind) -> &'static str {
    match value {
        SemanticKind::Numeric => "numeric",
        SemanticKind::Boolean => "boolean",
        SemanticKind::CumulativeCounter => "cumulative_counter",
        SemanticKind::Alarm => "alarm",
    }
}

fn parse_semantic_kind(value: &str) -> Result<SemanticKind, StorageError> {
    match value {
        "numeric" => Ok(SemanticKind::Numeric),
        "boolean" => Ok(SemanticKind::Boolean),
        "cumulative_counter" => Ok(SemanticKind::CumulativeCounter),
        "alarm" => Ok(SemanticKind::Alarm),
        _ => Err(StorageError::InvalidSemantic(
            "database contains an invalid semantic kind".into(),
        )),
    }
}

fn adapter_kind(value: SemanticKind) -> ObservationKind {
    match value {
        SemanticKind::Numeric => ObservationKind::Numeric,
        SemanticKind::Boolean => ObservationKind::Boolean,
        SemanticKind::CumulativeCounter => ObservationKind::CumulativeValue,
        SemanticKind::Alarm => ObservationKind::Alarm,
    }
}

fn prefixed_uuid(prefix: &str) -> String {
    format!("{prefix}{}", Uuid::new_v4().simple())
}

fn external_id(prefix: &str) -> String {
    format!("{prefix}{}", Uuid::new_v4().simple())
}

fn deterministic_export_id(route_id: &str, observation_id: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(route_id.as_bytes());
    digest.update([0]);
    digest.update(observation_id.as_bytes());
    format!("{:x}", digest.finalize())
}

fn transform_error_code(error: AdapterError) -> &'static str {
    match error {
        AdapterError::InvalidObservation => "invalid_observation",
        AdapterError::InvalidDescriptor
        | AdapterError::InvalidConfiguration
        | AdapterError::UnsupportedObservation
        | AdapterError::InvalidPublication
        | AdapterError::TransformFailed => "transform_failed",
    }
}

#[derive(Clone)]
struct RuleInventory {
    rule_id: String,
    signal_ref: String,
    edge_node_id: String,
    kind: SemanticKind,
}

fn compatible_modes(
    registration: &OutputAdapterRegistration,
    kind: SemanticKind,
) -> Vec<&'static str> {
    let kind = adapter_kind(kind);
    registration
        .adapter
        .descriptor()
        .modes
        .iter()
        .filter(|mode| mode.accepts.contains(&kind))
        .map(|mode| mode.key)
        .collect()
}

fn identity_scope_key(
    registration: &OutputAdapterRegistration,
    rule: &RuleInventory,
    mode: &str,
) -> String {
    match registration.profile_policy.identity_policy().scope {
        IdentityScope::RuleMode => format!("rule:{}:mode:{mode}", rule.rule_id),
        IdentityScope::Signal => format!("signal:{}", rule.signal_ref),
    }
}

fn row_to_binding<R: Row>(row: R) -> Result<OutputBinding, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    Option<String>: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
{
    let state: String = row.try_get("state")?;
    Ok(OutputBinding {
        binding_id: row.try_get("binding_id")?,
        rule_id: row.try_get("rule_id")?,
        external_id: row.try_get("external_id")?,
        mode: row.try_get("mode")?,
        active: state == "active",
        needs_configuration: state == "needs_configuration",
        ineligible_reason: row.try_get("ineligible_reason")?,
    })
}

fn row_to_semantic_rule<R: Row>(row: R) -> Result<SemanticRule, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    i64: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    bool: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
{
    Ok(SemanticRule {
        rule_id: row.try_get("rule_id")?,
        signal_ref: row.try_get("signal_ref")?,
        edge_node_id: row.try_get("edge_node_id")?,
        series_key: row.try_get("series_key")?,
        display_name: row.try_get("display_name")?,
        kind: parse_semantic_kind(&row.try_get::<String, _>("kind")?)?,
        series_id: row.try_get("series_id")?,
        revision: row.try_get("revision")?,
        active: row.try_get("active")?,
    })
}

struct ProjectionCandidate {
    rule_id: String,
    signal_ref: String,
    edge_node_id: String,
    ledger_epoch: String,
    pub_seq: i64,
    received_at: i64,
    record_json: Vec<u8>,
    current_revision: i64,
    current_series_id: String,
    current_spec: RuleSpec,
    current_calibration_revision: i64,
    current_scale: f64,
    current_offset: f64,
}

struct ProducedObservation {
    observation_id: String,
    rule_id: String,
    revision: i64,
    calibration_revision: i64,
    series_id: String,
    sequence: u64,
    kind: SemanticKind,
    value_json: Vec<u8>,
    value: ObservationValue,
    reading: Option<f64>,
    signal_ref: String,
    edge_node_id: String,
    ledger_epoch: String,
    source_pub_seq: i64,
    observed_at: i64,
    created_at: i64,
}

#[derive(Default)]
struct RouteRetry {
    attempted: bool,
    publications: usize,
}

fn decode_measurement(
    candidate: &ProjectionCandidate,
) -> Result<MeasurementRecord, StorageError> {
    let record: MeasurementRecord = serde_json::from_slice(&candidate.record_json)
        .map_err(|error| StorageError::InvalidSemantic(error.to_string()))?;
    if record.family != "measurement"
        || record.schema_version != 1
        || record.epoch != candidate.ledger_epoch
        || record.pub_seq != candidate.pub_seq
        || record.series_key.is_empty()
        || record.values.len() != 1
        || !record.values[0].is_finite()
        || record.event_time < 0
        || record.received_at < 0
        || record.event_time_source.is_empty()
        || record.time_source.is_empty()
        || record.time_quality.is_empty()
        || record.device_time.is_some_and(|value| value < 0)
    {
        return Err(StorageError::InvalidSemantic(
            "raw measurement is not projectable".into(),
        ));
    }
    Ok(record)
}

fn runtime_state<R: Row>(row: &R) -> Result<(EvaluationState, u64), StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    bool: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    i64: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
{
    let sequence: i64 = row.try_get("next_sequence")?;
    Ok((
        EvaluationState {
            initialized: row.try_get("initialized")?,
            active: row.try_get("detector_active")?,
            counter: row.try_get("counter")?,
            pending: row.try_get("pending")?,
            pending_active: row.try_get("pending_active")?,
            pending_since: row.try_get("pending_since")?,
        },
        u64::try_from(sequence)
            .map_err(|_| StorageError::InvalidSemantic("invalid next sequence".into()))?,
    ))
}

fn produced_observation(
    candidate: &ProjectionCandidate,
    record: &MeasurementRecord,
    evaluation: &crate::semantics::Evaluation,
    sequence: u64,
) -> Result<ProducedObservation, StorageError> {
    let (value, reading) = match candidate.current_spec.kind {
        SemanticKind::Numeric => (
            ObservationValue::Numeric(evaluation.number.ok_or_else(|| {
                StorageError::InvalidSemantic("numeric evaluation emitted no number".into())
            })?),
            None,
        ),
        SemanticKind::Boolean => (
            ObservationValue::Boolean(evaluation.boolean.ok_or_else(|| {
                StorageError::InvalidSemantic("boolean evaluation emitted no state".into())
            })?),
            None,
        ),
        SemanticKind::CumulativeCounter => (
            ObservationValue::CumulativeValue(u64::try_from(evaluation.integer.ok_or_else(
                || StorageError::InvalidSemantic("counter evaluation emitted no value".into()),
            )?)
            .map_err(|_| StorageError::InvalidSemantic("negative counter".into()))?),
            None,
        ),
        SemanticKind::Alarm => {
            let active = evaluation.boolean.ok_or_else(|| {
                StorageError::InvalidSemantic("alarm evaluation emitted no state".into())
            })?;
            (
                ObservationValue::Alarm {
                    active,
                    reading: Some(evaluation.calibrated),
                },
                Some(evaluation.calibrated),
            )
        }
    };
    let value_json = serde_json::to_vec(&match value {
        ObservationValue::Numeric(value) => serde_json::json!(value),
        ObservationValue::Boolean(value) => serde_json::json!(value),
        ObservationValue::CumulativeValue(value) => serde_json::json!(value),
        ObservationValue::Alarm { active, .. } => serde_json::json!(active),
    })
    .map_err(|error| StorageError::InvalidSemantic(error.to_string()))?;
    Ok(ProducedObservation {
        observation_id: Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!(
                "semantic-v3:{}:{}:{}",
                candidate.rule_id, candidate.ledger_epoch, candidate.pub_seq
            )
            .as_bytes(),
        )
        .to_string(),
        rule_id: candidate.rule_id.clone(),
        revision: candidate.current_revision,
        calibration_revision: candidate.current_calibration_revision,
        series_id: candidate.current_series_id.clone(),
        sequence,
        kind: candidate.current_spec.kind,
        value_json,
        value,
        reading,
        signal_ref: candidate.signal_ref.clone(),
        edge_node_id: candidate.edge_node_id.clone(),
        ledger_epoch: candidate.ledger_epoch.clone(),
        source_pub_seq: candidate.pub_seq,
        observed_at: record.event_time,
        created_at: record.received_at,
    })
}

fn adapter_observation(
    value: &ProducedObservation,
) -> Result<Observation, StorageError> {
    Observation::new(
        &value.observation_id,
        &value.series_id,
        value.sequence,
        value.observed_at,
        value.value.clone(),
    )
    .map_err(|error| StorageError::InvalidSemantic(error.to_string()))
}

fn sqlite_row_to_observation(
    row: sqlx::sqlite::SqliteRow,
) -> Result<SemanticObservation, StorageError> {
    let kind = parse_semantic_kind(&row.try_get::<String, _>("kind")?)?;
    let value: Vec<u8> = row.try_get("value_json")?;
    decode_stored_observation(
        row.try_get("observation_id")?,
        row.try_get("rule_id")?,
        row.try_get("series_id")?,
        row.try_get("sequence")?,
        kind,
        &value,
        row.try_get("reading")?,
        row.try_get("observed_at")?,
    )
}

fn postgres_row_to_observation(
    row: sqlx::postgres::PgRow,
) -> Result<SemanticObservation, StorageError> {
    let kind = parse_semantic_kind(&row.try_get::<String, _>("kind")?)?;
    let value: String = row.try_get("value_json")?;
    decode_stored_observation(
        row.try_get("observation_id")?,
        row.try_get("rule_id")?,
        row.try_get("series_id")?,
        row.try_get("sequence")?,
        kind,
        value.as_bytes(),
        row.try_get("reading")?,
        row.try_get("observed_at")?,
    )
}

#[allow(clippy::too_many_arguments)]
fn decode_stored_observation(
    observation_id: String,
    rule_id: String,
    series_id: String,
    sequence: i64,
    kind: SemanticKind,
    value: &[u8],
    reading: Option<f64>,
    observed_at: i64,
) -> Result<SemanticObservation, StorageError> {
    let value: Value = serde_json::from_slice(value)
        .map_err(|error| StorageError::InvalidSemantic(error.to_string()))?;
    let value = match kind {
        SemanticKind::Numeric => ObservationValue::Numeric(
            value
                .as_f64()
                .ok_or_else(|| StorageError::InvalidSemantic("invalid numeric value".into()))?,
        ),
        SemanticKind::Boolean => ObservationValue::Boolean(
            value
                .as_bool()
                .ok_or_else(|| StorageError::InvalidSemantic("invalid boolean value".into()))?,
        ),
        SemanticKind::CumulativeCounter => ObservationValue::CumulativeValue(
            value
                .as_u64()
                .ok_or_else(|| StorageError::InvalidSemantic("invalid counter value".into()))?,
        ),
        SemanticKind::Alarm => ObservationValue::Alarm {
            active: value
                .as_bool()
                .ok_or_else(|| StorageError::InvalidSemantic("invalid alarm value".into()))?,
            reading,
        },
    };
    Ok(SemanticObservation {
        observation_id,
        rule_id,
        series_id,
        sequence: u64::try_from(sequence)
            .map_err(|_| StorageError::InvalidSemantic("invalid observation sequence".into()))?,
        observed_at,
        value,
    })
}
