use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, QueryBuilder, Row, Sqlite};

use super::{Storage, StorageError, StorageInner};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawHistoryQuery {
    pub from: i64,
    pub to: i64,
    pub limit: usize,
    pub cursor: Option<String>,
    pub signal_ref: Option<String>,
    pub edge_node_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRawHistoryRow {
    pub received_at: i64,
    pub edge_node_id: String,
    pub ledger_epoch: String,
    pub pub_seq: i64,
    pub signal_ref: String,
    pub series_key: String,
    pub display_name: String,
    pub unit: String,
    pub decimal_places: i32,
    pub display_value_kind: String,
    pub record_json: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawHistoryPage {
    pub rows: Vec<StoredRawHistoryRow>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredSemanticHistoryRow {
    pub observed_at: i64,
    pub processed_at: i64,
    pub edge_node_id: String,
    pub signal_ref: String,
    pub signal_name: String,
    pub rule_name: String,
    pub kind: String,
    pub value_json: Vec<u8>,
    pub unit: String,
    pub series_id: String,
    pub sequence: i64,
    pub observation_id: String,
    pub rule_revision: i64,
    pub calibration_revision: i64,
    pub source_pub_seq: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryBucket {
    pub bucket_start: i64,
    pub minimum: f64,
    pub average: f64,
    pub maximum: f64,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawCursor {
    received_at: i64,
    edge_node_id: String,
    ledger_epoch: String,
    pub_seq: i64,
}

impl Storage {
    pub async fn query_raw_history(
        &self,
        query: RawHistoryQuery,
    ) -> Result<RawHistoryPage, StorageError> {
        if query.from < 0 || query.to <= query.from || !(1..=100_001).contains(&query.limit) {
            return Err(StorageError::InvalidHistory(
                "invalid range or limit".into(),
            ));
        }
        let cursor = query.cursor.as_deref().map(decode_cursor).transpose()?;
        let requested = i64::try_from(query.limit)
            .map_err(|_| StorageError::InvalidHistory("limit is too large".into()))?;
        let fetch_limit = requested.saturating_add(1);
        let mut rows = match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut sql = QueryBuilder::<Sqlite>::new(
                    "SELECT raw.received_at,raw.edge_node_id,raw.ledger_epoch,raw.pub_seq,\
                     COALESCE(signal.signal_ref,'' ) signal_ref,\
                     COALESCE(json_extract(raw.record_json,'$.series_key'),'' ) series_key,\
                     COALESCE(profile.display_name,descriptor.measurement_key,\
                       json_extract(raw.record_json,'$.series_key'),'' ) display_name,\
                     COALESCE(profile.display_unit,descriptor.unit,'' ) unit,\
                     COALESCE(profile.decimal_places,0) decimal_places,\
                     COALESCE(profile.display_value_kind,descriptor.value_type,'' ) display_value_kind,\
                     raw.record_json FROM raw_records raw \
                     LEFT JOIN inventory_signals signal ON signal.edge_node_id=raw.edge_node_id \
                       AND signal.series_key=json_extract(raw.record_json,'$.series_key') \
                     LEFT JOIN descriptor_signals descriptor ON descriptor.edge_node_id=raw.edge_node_id \
                       AND descriptor.series_key=signal.series_key \
                     LEFT JOIN signal_profiles profile ON profile.edge_node_id=raw.edge_node_id \
                       AND profile.series_key=signal.series_key WHERE raw.received_at>=",
                );
                sql.push_bind(query.from)
                    .push(" AND raw.received_at<")
                    .push_bind(query.to);
                append_raw_filters(&mut sql, &query, cursor.as_ref());
                sql.push(
                    " ORDER BY raw.received_at DESC,raw.edge_node_id DESC,\
                     raw.ledger_epoch DESC,raw.pub_seq DESC LIMIT ",
                )
                .push_bind(fetch_limit);
                sql.build()
                    .fetch_all(pool)
                    .await?
                    .into_iter()
                    .map(|row| raw_row(&row))
                    .collect::<Result<Vec<_>, _>>()?
            }
            StorageInner::Postgres { pool, .. } => {
                let mut sql = QueryBuilder::<Postgres>::new(
                    "SELECT raw.received_at,raw.edge_node_id,raw.ledger_epoch,raw.pub_seq,\
                     COALESCE(signal.signal_ref,'' ) signal_ref,\
                     COALESCE(convert_from(raw.record_json,'UTF8')::jsonb->>'series_key','') series_key,\
                     COALESCE(profile.display_name,descriptor.measurement_key,\
                       convert_from(raw.record_json,'UTF8')::jsonb->>'series_key','') display_name,\
                     COALESCE(profile.display_unit,descriptor.unit,'' ) unit,\
                     COALESCE(profile.decimal_places,0) decimal_places,\
                     COALESCE(profile.display_value_kind,descriptor.value_type,'' ) display_value_kind,\
                     raw.record_json FROM raw_records raw \
                     LEFT JOIN inventory_signals signal ON signal.edge_node_id=raw.edge_node_id \
                       AND signal.series_key=convert_from(raw.record_json,'UTF8')::jsonb->>'series_key' \
                     LEFT JOIN descriptor_signals descriptor ON descriptor.edge_node_id=raw.edge_node_id \
                       AND descriptor.series_key=signal.series_key \
                     LEFT JOIN signal_profiles profile ON profile.edge_node_id=raw.edge_node_id \
                       AND profile.series_key=signal.series_key WHERE raw.received_at>=",
                );
                sql.push_bind(query.from)
                    .push(" AND raw.received_at<")
                    .push_bind(query.to);
                append_raw_filters(&mut sql, &query, cursor.as_ref());
                sql.push(
                    " ORDER BY raw.received_at DESC,raw.edge_node_id DESC,\
                     raw.ledger_epoch DESC,raw.pub_seq DESC LIMIT ",
                )
                .push_bind(fetch_limit);
                sql.build()
                    .fetch_all(pool)
                    .await?
                    .into_iter()
                    .map(|row| raw_row(&row))
                    .collect::<Result<Vec<_>, _>>()?
            }
        };
        let has_more = rows.len() > query.limit;
        rows.truncate(query.limit);
        let next_cursor = if has_more {
            rows.last().map(encode_row_cursor).transpose()?
        } else {
            None
        };
        Ok(RawHistoryPage {
            rows,
            next_cursor,
            has_more,
        })
    }

    pub async fn query_semantic_history(
        &self,
        from: i64,
        to: i64,
        limit: usize,
        signal_ref: Option<&str>,
        edge_node_id: Option<&str>,
    ) -> Result<Vec<StoredSemanticHistoryRow>, StorageError> {
        if from < 0 || to <= from || !(1..=100_001).contains(&limit) {
            return Err(StorageError::InvalidHistory(
                "invalid range or limit".into(),
            ));
        }
        let rows = match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut sql = QueryBuilder::<Sqlite>::new(semantic_select(false));
                append_semantic_filters(&mut sql, from, to, signal_ref, edge_node_id);
                sql.push(" ORDER BY observation.observed_at DESC,observation.observation_row_id DESC LIMIT ")
                    .push_bind(i64::try_from(limit).unwrap_or(100_001));
                sql.build()
                    .fetch_all(pool)
                    .await?
                    .into_iter()
                    .map(|row| semantic_row(&row, false))
                    .collect::<Result<Vec<_>, _>>()?
            }
            StorageInner::Postgres { pool, .. } => {
                let mut sql = QueryBuilder::<Postgres>::new(semantic_select(true));
                append_semantic_filters(&mut sql, from, to, signal_ref, edge_node_id);
                sql.push(" ORDER BY observation.observed_at DESC,observation.observation_row_id DESC LIMIT ")
                    .push_bind(i64::try_from(limit).unwrap_or(100_001));
                sql.build()
                    .fetch_all(pool)
                    .await?
                    .into_iter()
                    .map(|row| semantic_row(&row, true))
                    .collect::<Result<Vec<_>, _>>()?
            }
        };
        Ok(rows)
    }

    pub async fn query_history_series(
        &self,
        signal_ref: &str,
        from: i64,
        to: i64,
        bucket_ms: i64,
    ) -> Result<Vec<HistoryBucket>, StorageError> {
        if signal_ref.is_empty() || from < 0 || to <= from || bucket_ms <= 0 {
            return Err(StorageError::InvalidHistory("invalid series query".into()));
        }
        let rows = match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => sqlx::query(
                "SELECT ((raw.received_at-?)/?)*?+? bucket_start,\
                 MIN(CAST(json_extract(raw.record_json,'$.values[0]') AS REAL)) minimum,\
                 AVG(CAST(json_extract(raw.record_json,'$.values[0]') AS REAL)) average,\
                 MAX(CAST(json_extract(raw.record_json,'$.values[0]') AS REAL)) maximum,\
                 COUNT(*) count FROM raw_records raw JOIN inventory_signals signal \
                 ON signal.edge_node_id=raw.edge_node_id \
                 AND signal.series_key=json_extract(raw.record_json,'$.series_key') \
                 WHERE signal.signal_ref=? AND raw.received_at>=? AND raw.received_at<? \
                 AND json_type(raw.record_json,'$.values[0]') IN ('integer','real','true','false') \
                 GROUP BY bucket_start ORDER BY bucket_start",
            )
            .bind(from)
            .bind(bucket_ms)
            .bind(bucket_ms)
            .bind(from)
            .bind(signal_ref)
            .bind(from)
            .bind(to)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|row| bucket_row(&row))
            .collect::<Result<Vec<_>, _>>()?,
            StorageInner::Postgres { pool, .. } => sqlx::query(
                "SELECT ((raw.received_at-$1)/$2)*$2+$1 bucket_start,\
                 MIN(CASE convert_from(raw.record_json,'UTF8')::jsonb->'values'->0 \
                     WHEN 'true'::jsonb THEN 1 WHEN 'false'::jsonb THEN 0 \
                     ELSE (convert_from(raw.record_json,'UTF8')::jsonb->'values'->>0)::double precision END) minimum,\
                 AVG(CASE convert_from(raw.record_json,'UTF8')::jsonb->'values'->0 \
                     WHEN 'true'::jsonb THEN 1 WHEN 'false'::jsonb THEN 0 \
                     ELSE (convert_from(raw.record_json,'UTF8')::jsonb->'values'->>0)::double precision END) average,\
                 MAX(CASE convert_from(raw.record_json,'UTF8')::jsonb->'values'->0 \
                     WHEN 'true'::jsonb THEN 1 WHEN 'false'::jsonb THEN 0 \
                     ELSE (convert_from(raw.record_json,'UTF8')::jsonb->'values'->>0)::double precision END) maximum,\
                 COUNT(*)::bigint count FROM raw_records raw JOIN inventory_signals signal \
                 ON signal.edge_node_id=raw.edge_node_id \
                 AND signal.series_key=convert_from(raw.record_json,'UTF8')::jsonb->>'series_key' \
                 WHERE signal.signal_ref=$3 AND raw.received_at>=$1 AND raw.received_at<$4 \
                 AND jsonb_typeof(convert_from(raw.record_json,'UTF8')::jsonb->'values'->0) IN ('number','boolean') \
                 GROUP BY bucket_start ORDER BY bucket_start",
            )
            .bind(from)
            .bind(bucket_ms)
            .bind(signal_ref)
            .bind(to)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|row| bucket_row(&row))
            .collect::<Result<Vec<_>, _>>()?,
        };
        Ok(rows)
    }

    pub async fn query_semantic_history_series(
        &self,
        rule_id: &str,
        from: i64,
        to: i64,
        bucket_ms: i64,
    ) -> Result<(Vec<HistoryBucket>, Option<(i64, Vec<u8>)>), StorageError> {
        if rule_id.is_empty() || from < 0 || to <= from || bucket_ms <= 0 {
            return Err(StorageError::InvalidHistory(
                "invalid semantic series query".into(),
            ));
        }
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let buckets = sqlx::query(
                    "SELECT ((raw.received_at-?)/?)*?+? bucket_start,\
                     MIN(CAST(json_extract(observation.value_json,'$') AS REAL)) minimum,\
                     AVG(CAST(json_extract(observation.value_json,'$') AS REAL)) average,\
                     MAX(CAST(json_extract(observation.value_json,'$') AS REAL)) maximum,\
                     COUNT(*) count FROM semantic_observations observation JOIN raw_records raw \
                     ON raw.edge_node_id=observation.edge_node_id \
                     AND raw.ledger_epoch=observation.ledger_epoch \
                     AND raw.pub_seq=observation.source_pub_seq \
                     WHERE observation.rule_id=? AND raw.received_at>=? AND raw.received_at<? \
                     AND json_type(observation.value_json,'$') IN ('integer','real','true','false') \
                     GROUP BY bucket_start ORDER BY bucket_start",
                )
                .bind(from)
                .bind(bucket_ms)
                .bind(bucket_ms)
                .bind(from)
                .bind(rule_id)
                .bind(from)
                .bind(to)
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(|row| bucket_row(&row))
                .collect::<Result<Vec<_>, _>>()?;
                let latest = sqlx::query(
                    "SELECT raw.received_at,observation.value_json FROM semantic_observations observation \
                     JOIN raw_records raw ON raw.edge_node_id=observation.edge_node_id \
                     AND raw.ledger_epoch=observation.ledger_epoch \
                     AND raw.pub_seq=observation.source_pub_seq WHERE observation.rule_id=? \
                     ORDER BY raw.received_at DESC,observation.observation_row_id DESC LIMIT 1",
                )
                .bind(rule_id)
                .fetch_optional(pool)
                .await?
                .map(|row| {
                    Ok::<_, StorageError>((
                        row.try_get("received_at")?,
                        row.try_get("value_json")?,
                    ))
                })
                .transpose()?;
                Ok((buckets, latest))
            }
            StorageInner::Postgres { pool, .. } => {
                let buckets = sqlx::query(
                    "SELECT ((raw.received_at-$1)/$2)*$2+$1 bucket_start,\
                     MIN(CASE observation.value_json WHEN 'true'::jsonb THEN 1 \
                         WHEN 'false'::jsonb THEN 0 ELSE (observation.value_json#>>'{}')::double precision END) minimum,\
                     AVG(CASE observation.value_json WHEN 'true'::jsonb THEN 1 \
                         WHEN 'false'::jsonb THEN 0 ELSE (observation.value_json#>>'{}')::double precision END) average,\
                     MAX(CASE observation.value_json WHEN 'true'::jsonb THEN 1 \
                         WHEN 'false'::jsonb THEN 0 ELSE (observation.value_json#>>'{}')::double precision END) maximum,\
                     COUNT(*)::bigint count FROM semantic_observations observation JOIN raw_records raw \
                     ON raw.edge_node_id=observation.edge_node_id \
                     AND raw.ledger_epoch=observation.ledger_epoch \
                     AND raw.pub_seq=observation.source_pub_seq \
                     WHERE observation.rule_id=$3 AND raw.received_at>=$1 AND raw.received_at<$4 \
                     AND jsonb_typeof(observation.value_json) IN ('number','boolean') \
                     GROUP BY bucket_start ORDER BY bucket_start",
                )
                .bind(from)
                .bind(bucket_ms)
                .bind(rule_id)
                .bind(to)
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(|row| bucket_row(&row))
                .collect::<Result<Vec<_>, _>>()?;
                let latest = sqlx::query(
                    "SELECT raw.received_at,observation.value_json::text value_json \
                     FROM semantic_observations observation JOIN raw_records raw \
                     ON raw.edge_node_id=observation.edge_node_id \
                     AND raw.ledger_epoch=observation.ledger_epoch \
                     AND raw.pub_seq=observation.source_pub_seq WHERE observation.rule_id=$1 \
                     ORDER BY raw.received_at DESC,observation.observation_row_id DESC LIMIT 1",
                )
                .bind(rule_id)
                .fetch_optional(pool)
                .await?
                .map(|row| {
                    Ok::<_, StorageError>((
                        row.try_get("received_at")?,
                        row.try_get::<String, _>("value_json")?.into_bytes(),
                    ))
                })
                .transpose()?;
                Ok((buckets, latest))
            }
        }
    }
}

fn raw_row<R>(row: &R) -> Result<StoredRawHistoryRow, StorageError>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    i64: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    i32: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    String: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    Vec<u8>: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
{
    Ok(StoredRawHistoryRow {
        received_at: row.try_get("received_at")?,
        edge_node_id: row.try_get("edge_node_id")?,
        ledger_epoch: row.try_get("ledger_epoch")?,
        pub_seq: row.try_get("pub_seq")?,
        signal_ref: row.try_get("signal_ref")?,
        series_key: row.try_get("series_key")?,
        display_name: row.try_get("display_name")?,
        unit: row.try_get("unit")?,
        decimal_places: row.try_get("decimal_places")?,
        display_value_kind: row.try_get("display_value_kind")?,
        record_json: row.try_get("record_json")?,
    })
}

fn semantic_row<R>(row: &R, postgres: bool) -> Result<StoredSemanticHistoryRow, StorageError>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    i64: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    String: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    Vec<u8>: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
{
    let value_json = if postgres {
        row.try_get::<String, _>("value_json")?.into_bytes()
    } else {
        row.try_get("value_json")?
    };
    Ok(StoredSemanticHistoryRow {
        observed_at: row.try_get("observed_at")?,
        processed_at: row.try_get("processed_at")?,
        edge_node_id: row.try_get("edge_node_id")?,
        signal_ref: row.try_get("signal_ref")?,
        signal_name: row.try_get("signal_name")?,
        rule_name: row.try_get("rule_name")?,
        kind: row.try_get("kind")?,
        value_json,
        unit: row.try_get("unit")?,
        series_id: row.try_get("series_id")?,
        sequence: row.try_get("sequence")?,
        observation_id: row.try_get("observation_id")?,
        rule_revision: row.try_get("rule_revision")?,
        calibration_revision: row.try_get("calibration_revision")?,
        source_pub_seq: row.try_get("source_pub_seq")?,
    })
}

fn bucket_row<R>(row: &R) -> Result<HistoryBucket, StorageError>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    i64: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    f64: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
{
    Ok(HistoryBucket {
        bucket_start: row.try_get("bucket_start")?,
        minimum: row.try_get("minimum")?,
        average: row.try_get("average")?,
        maximum: row.try_get("maximum")?,
        count: row.try_get("count")?,
    })
}

fn append_raw_filters<'a, DB>(
    sql: &mut QueryBuilder<'a, DB>,
    query: &'a RawHistoryQuery,
    cursor: Option<&'a RawCursor>,
) where
    DB: sqlx::Database,
    String: sqlx::Encode<'a, DB> + sqlx::Type<DB>,
    i64: sqlx::Encode<'a, DB> + sqlx::Type<DB>,
{
    if let Some(edge_node_id) = &query.edge_node_id {
        sql.push(" AND raw.edge_node_id=")
            .push_bind(edge_node_id.clone());
    }
    if let Some(signal_ref) = &query.signal_ref {
        sql.push(" AND signal.signal_ref=")
            .push_bind(signal_ref.clone());
    }
    if let Some(cursor) = cursor {
        sql.push(" AND (raw.received_at,raw.edge_node_id,raw.ledger_epoch,raw.pub_seq)<(")
            .push_bind(cursor.received_at)
            .push(",")
            .push_bind(cursor.edge_node_id.clone())
            .push(",")
            .push_bind(cursor.ledger_epoch.clone())
            .push(",")
            .push_bind(cursor.pub_seq)
            .push(")");
    }
}

fn semantic_select(postgres: bool) -> &'static str {
    if postgres {
        "SELECT observation.observed_at,observation.created_at processed_at,\
         observation.edge_node_id,observation.signal_ref,\
         COALESCE(profile.display_name,signal.series_key) signal_name,rule.display_name rule_name,\
         observation.kind,observation.value_json::text value_json,\
         COALESCE(profile.display_unit,'') unit,observation.series_id,observation.sequence,\
         observation.observation_id,observation.revision rule_revision,\
         observation.calibration_revision,observation.source_pub_seq \
         FROM semantic_observations observation JOIN semantic_rules rule ON rule.rule_id=observation.rule_id \
         JOIN semantic_signals signal ON signal.signal_ref=observation.signal_ref \
         LEFT JOIN signal_profiles profile ON profile.edge_node_id=signal.edge_node_id \
           AND profile.series_key=signal.series_key WHERE observation.observed_at>="
    } else {
        "SELECT observation.observed_at,observation.created_at processed_at,\
         observation.edge_node_id,observation.signal_ref,\
         COALESCE(profile.display_name,signal.series_key) signal_name,rule.display_name rule_name,\
         observation.kind,observation.value_json,\
         COALESCE(profile.display_unit,'') unit,observation.series_id,observation.sequence,\
         observation.observation_id,observation.revision rule_revision,\
         observation.calibration_revision,observation.source_pub_seq \
         FROM semantic_observations observation JOIN semantic_rules rule ON rule.rule_id=observation.rule_id \
         JOIN semantic_signals signal ON signal.signal_ref=observation.signal_ref \
         LEFT JOIN signal_profiles profile ON profile.edge_node_id=signal.edge_node_id \
           AND profile.series_key=signal.series_key WHERE observation.observed_at>="
    }
}

fn append_semantic_filters<'a, DB>(
    sql: &mut QueryBuilder<'a, DB>,
    from: i64,
    to: i64,
    signal_ref: Option<&'a str>,
    edge_node_id: Option<&'a str>,
) where
    DB: sqlx::Database,
    &'a str: sqlx::Encode<'a, DB> + sqlx::Type<DB>,
    i64: sqlx::Encode<'a, DB> + sqlx::Type<DB>,
{
    sql.push_bind(from)
        .push(" AND observation.observed_at<")
        .push_bind(to);
    if let Some(signal_ref) = signal_ref {
        sql.push(" AND observation.signal_ref=")
            .push_bind(signal_ref);
    }
    if let Some(edge_node_id) = edge_node_id {
        sql.push(" AND observation.edge_node_id=")
            .push_bind(edge_node_id);
    }
}

fn encode_row_cursor(row: &StoredRawHistoryRow) -> Result<String, StorageError> {
    serde_json::to_vec(&RawCursor {
        received_at: row.received_at,
        edge_node_id: row.edge_node_id.clone(),
        ledger_epoch: row.ledger_epoch.clone(),
        pub_seq: row.pub_seq,
    })
    .map(|value| URL_SAFE_NO_PAD.encode(value))
    .map_err(|error| StorageError::InvalidHistory(error.to_string()))
}

fn decode_cursor(value: &str) -> Result<RawCursor, StorageError> {
    URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| StorageError::InvalidHistory("cursor is malformed".into()))
        .and_then(|value| {
            serde_json::from_slice(&value)
                .map_err(|_| StorageError::InvalidHistory("cursor is malformed".into()))
        })
}
