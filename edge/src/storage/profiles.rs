use serde_json::json;
use sqlx::{Postgres, QueryBuilder, Row, postgres::PgRow, sqlite::SqliteRow};

use crate::application::profiles::{
    DeviceProfile, DeviceProfileInput, InventoryDevice, InventorySignal, SignalProfile,
    SignalProfileInput,
};

use super::{
    AuditActor, Storage, StorageError, StorageInner, StoredPreviewInput,
    auth::{insert_audit_postgres, insert_audit_sqlite},
};

#[doc(hidden)]
pub const SQLITE_SIGNAL_IDENTITY_SQL: &str =
    "SELECT edge_node_id,series_key FROM inventory_signals WHERE signal_ref=?";

#[doc(hidden)]
pub const POSTGRES_SIGNAL_IDENTITY_SQL: &str =
    "SELECT edge_node_id,series_key FROM inventory_signals WHERE signal_ref=$1";

#[doc(hidden)]
pub const SQLITE_RECENT_SIGNAL_INPUTS_SQL: &str = "SELECT received_at,record_json FROM raw_records \
     WHERE edge_node_id=? AND series_key=? \
     ORDER BY received_at DESC,ledger_epoch DESC,pub_seq DESC LIMIT ?";

#[doc(hidden)]
// The digest narrows the index lookup; the following full-key predicate preserves exact identity.
pub const POSTGRES_RECENT_SIGNAL_INPUTS_SQL: &str = "SELECT received_at,record_json FROM raw_records \
     WHERE edge_node_id=$1 AND md5(series_key)=md5($2) AND series_key=$2 \
     ORDER BY received_at DESC,ledger_epoch DESC,pub_seq DESC LIMIT $3";

/// Returns one latest receipt per deliberately bounded current signal
/// inventory. Callers ask for one extra row so they can report `64+` rather
/// than silently claiming a complete healthy inventory.
#[doc(hidden)]
pub const SQLITE_DIAGNOSTIC_SIGNAL_RECEIPTS_SQL: &str = "SELECT (SELECT raw.received_at \
    FROM raw_records AS raw WHERE raw.edge_node_id=activation.edge_node_id \
    AND raw.ledger_epoch=activation.ledger_epoch AND raw.series_key=signal.series_key \
    ORDER BY raw.received_at DESC,raw.pub_seq DESC LIMIT 1) AS received_at \
    FROM inventory_signals AS signal CROSS JOIN edge_node_activations AS activation \
    WHERE activation.edge_node_id=signal.edge_node_id AND activation.state='active' \
    AND EXISTS(SELECT 1 FROM descriptor_signals AS descriptor WHERE descriptor.edge_node_id=signal.edge_node_id \
      AND descriptor.series_key=signal.series_key AND descriptor.presence='current') \
    ORDER BY signal.edge_node_id,signal.series_key LIMIT ?";
#[doc(hidden)]
pub const POSTGRES_DIAGNOSTIC_SIGNAL_IDENTITIES_SQL: &str = "SELECT signal.edge_node_id,activation.ledger_epoch,signal.series_key \
    FROM inventory_signals AS signal JOIN edge_node_activations AS activation \
    ON activation.edge_node_id=signal.edge_node_id WHERE activation.state='active' \
    AND EXISTS(SELECT 1 FROM descriptor_signals AS descriptor WHERE descriptor.edge_node_id=signal.edge_node_id \
      AND descriptor.series_key=signal.series_key AND descriptor.presence='current') \
    ORDER BY signal.edge_node_id,signal.series_key LIMIT $1";

/// One direct-bound raw lookup used for every arm of the bounded PostgreSQL
/// diagnostic batch.  The fixed-width digest chooses the existing index and
/// the full-key predicate remains the collision-safe authority.
#[doc(hidden)]
pub const POSTGRES_DIAGNOSTIC_SIGNAL_RECEIPT_SQL: &str = "SELECT (SELECT raw.received_at \
    FROM raw_records AS raw WHERE raw.edge_node_id=$1 \
    AND raw.ledger_epoch=$2 AND md5(raw.series_key)=md5($3) AND raw.series_key=$3 \
    ORDER BY raw.received_at DESC,raw.pub_seq DESC LIMIT 1) AS received_at";

impl Storage {
    /// Bounded latest receipts only; this is intentionally not
    /// `inventory_signals`, whose unbounded result is suitable for pages but
    /// not a health check.
    pub async fn diagnostic_signal_receipts(
        &self,
        limit: i64,
    ) -> Result<Vec<Option<i64>>, StorageError> {
        if !(1..=65).contains(&limit) {
            return Err(StorageError::InvalidProfile(
                "diagnostic signal limit must be between 1 and 65".into(),
            ));
        }
        let rows = match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                sqlx::query_scalar(SQLITE_DIAGNOSTIC_SIGNAL_RECEIPTS_SQL)
                    .bind(limit)
                    .fetch_all(pool)
                    .await?
            }
            StorageInner::Postgres { pool, .. } => {
                let identities: Vec<(String, String, String)> =
                    sqlx::query_as(POSTGRES_DIAGNOSTIC_SIGNAL_IDENTITIES_SQL)
                        .bind(limit)
                        .fetch_all(pool)
                        .await?;
                postgres_diagnostic_signal_receipts(pool, &identities).await?
            }
        };
        Ok(rows)
    }

    pub async fn recent_signal_inputs(
        &self,
        signal_ref: &str,
        limit: i64,
    ) -> Result<Vec<StoredPreviewInput>, StorageError> {
        if !(1..=2_000).contains(&limit) {
            return Err(StorageError::InvalidProfile(
                "preview limit must be between 1 and 2000".into(),
            ));
        }
        let mut values = match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let identity: Option<(String, String)> = sqlx::query_as(SQLITE_SIGNAL_IDENTITY_SQL)
                    .bind(signal_ref)
                    .fetch_optional(pool)
                    .await?;
                let Some((edge_node_id, series_key)) = identity else {
                    return Ok(Vec::new());
                };
                sqlx::query(SQLITE_RECENT_SIGNAL_INPUTS_SQL)
                    .bind(edge_node_id)
                    .bind(series_key)
                    .bind(limit)
                    .fetch_all(pool)
                    .await?
                    .into_iter()
                    .map(|row| {
                        Ok(StoredPreviewInput {
                            received_at: row.try_get("received_at")?,
                            record_json: row.try_get("record_json")?,
                        })
                    })
                    .collect::<Result<Vec<_>, StorageError>>()?
            }
            StorageInner::Postgres { pool, .. } => {
                let identity: Option<(String, String)> =
                    sqlx::query_as(POSTGRES_SIGNAL_IDENTITY_SQL)
                        .bind(signal_ref)
                        .fetch_optional(pool)
                        .await?;
                let Some((edge_node_id, series_key)) = identity else {
                    return Ok(Vec::new());
                };
                sqlx::query(POSTGRES_RECENT_SIGNAL_INPUTS_SQL)
                    .bind(edge_node_id)
                    .bind(series_key)
                    .bind(limit)
                    .fetch_all(pool)
                    .await?
                    .into_iter()
                    .map(|row| {
                        Ok(StoredPreviewInput {
                            received_at: row.try_get("received_at")?,
                            record_json: row.try_get("record_json")?,
                        })
                    })
                    .collect::<Result<Vec<_>, StorageError>>()?
            }
        };
        values.reverse();
        Ok(values)
    }

    pub async fn inventory_devices(&self) -> Result<Vec<InventoryDevice>, StorageError> {
        let sql = "SELECT inventory.device_ref,inventory.edge_node_id,inventory.system_id,\
            COALESCE(descriptor.identifier,'' ) AS identifier,descriptor.state,\
            descriptor.presence,COALESCE(descriptor.model_id,'') AS model_id,\
            COALESCE(profile.display_name,'') AS display_name,\
            COALESCE(profile.location,'') AS location,profile.revision AS profile_revision \
            FROM inventory_devices AS inventory JOIN descriptor_devices AS descriptor \
            ON descriptor.edge_node_id=inventory.edge_node_id \
            AND descriptor.system_id=inventory.system_id LEFT JOIN device_profiles AS profile \
            ON profile.edge_node_id=inventory.edge_node_id AND profile.system_id=inventory.system_id \
            ORDER BY inventory.edge_node_id,inventory.system_id";
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => sqlx::query(sql)
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(device_sqlite)
                .collect(),
            StorageInner::Postgres { pool, .. } => sqlx::query(sql)
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(device_postgres)
                .collect(),
        }
    }

    pub async fn inventory_signals(&self) -> Result<Vec<InventorySignal>, StorageError> {
        let sql = "SELECT signal.signal_ref,device.device_ref,signal.edge_node_id,\
            signal.series_key,signal.system_id,descriptor.measurement_key,descriptor.variant,\
            COALESCE(descriptor.unit,'') AS unit,descriptor.value_type,descriptor.presence,\
            COALESCE(profile.display_name,'') AS display_name,\
            COALESCE(profile.display_sensor_type,'') AS display_sensor_type,\
            COALESCE(profile.display_sensor_type_label,'') AS display_sensor_type_label,\
            COALESCE(profile.display_value_kind,'') AS display_value_kind,\
            COALESCE(profile.display_unit_mode,'') AS display_unit_mode,\
            COALESCE(profile.display_unit,'') AS display_unit,\
            COALESCE(profile.decimal_places,0) AS decimal_places,\
            profile.revision AS profile_revision FROM inventory_signals AS signal \
            JOIN inventory_devices AS device ON device.edge_node_id=signal.edge_node_id \
            AND device.system_id=signal.system_id JOIN descriptor_signals AS descriptor \
            ON descriptor.edge_node_id=signal.edge_node_id \
            AND descriptor.series_key=signal.series_key LEFT JOIN signal_profiles AS profile \
            ON profile.edge_node_id=signal.edge_node_id AND profile.series_key=signal.series_key \
            ORDER BY signal.edge_node_id,signal.series_key";
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => sqlx::query(sql)
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(signal_sqlite)
                .collect(),
            StorageInner::Postgres { pool, .. } => sqlx::query(sql)
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(signal_postgres)
                .collect(),
        }
    }

    pub async fn update_device_profile(
        &self,
        actor: AuditActor,
        device_ref: &str,
        input: DeviceProfileInput,
        expected_revision: Option<i64>,
        now: i64,
    ) -> Result<DeviceProfile, StorageError> {
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let identity: Option<(String, String)> = sqlx::query_as(
                    "SELECT edge_node_id,system_id FROM inventory_devices WHERE device_ref=?",
                )
                .bind(device_ref)
                .fetch_optional(&mut *tx)
                .await?;
                let (edge_node_id, system_id) = identity.ok_or(StorageError::ProfileNotFound)?;
                let current: Option<i64> = sqlx::query_scalar(
                    "SELECT revision FROM device_profiles WHERE edge_node_id=? AND system_id=?",
                )
                .bind(&edge_node_id)
                .bind(&system_id)
                .fetch_optional(&mut *tx)
                .await?;
                check_revision(current, expected_revision)?;
                let revision = current.unwrap_or(0) + 1;
                sqlx::query(
                    "INSERT INTO device_profiles(edge_node_id,system_id,display_name,location,\
                     revision,updated_at) VALUES(?,?,?,?,?,?) ON CONFLICT(edge_node_id,system_id) \
                     DO UPDATE SET display_name=excluded.display_name,location=excluded.location,\
                     revision=excluded.revision,updated_at=excluded.updated_at",
                )
                .bind(&edge_node_id)
                .bind(&system_id)
                .bind(&input.display_name)
                .bind(&input.location)
                .bind(revision)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                insert_audit_sqlite(
                    &mut tx,
                    &actor,
                    now,
                    "device_profile.update",
                    device_ref,
                    json!({"display_name":input.display_name,"location":input.location,
                        "revision":revision}),
                )
                .await?;
                tx.commit().await?;
                Ok(DeviceProfile {
                    device_ref: device_ref.into(),
                    display_name: input.display_name,
                    location: input.location,
                    revision,
                    updated_at: now,
                })
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                let identity: Option<(String, String)> = sqlx::query_as(
                    "SELECT edge_node_id,system_id FROM inventory_devices \
                     WHERE device_ref=$1 FOR UPDATE",
                )
                .bind(device_ref)
                .fetch_optional(&mut *tx)
                .await?;
                let (edge_node_id, system_id) = identity.ok_or(StorageError::ProfileNotFound)?;
                let current: Option<i64> = sqlx::query_scalar(
                    "SELECT revision FROM device_profiles WHERE edge_node_id=$1 AND system_id=$2",
                )
                .bind(&edge_node_id)
                .bind(&system_id)
                .fetch_optional(&mut *tx)
                .await?;
                check_revision(current, expected_revision)?;
                let revision = current.unwrap_or(0) + 1;
                sqlx::query(
                    "INSERT INTO device_profiles(edge_node_id,system_id,display_name,location,\
                     revision,updated_at) VALUES($1,$2,$3,$4,$5,$6) \
                     ON CONFLICT(edge_node_id,system_id) DO UPDATE SET \
                     display_name=excluded.display_name,location=excluded.location,\
                     revision=excluded.revision,updated_at=excluded.updated_at",
                )
                .bind(&edge_node_id)
                .bind(&system_id)
                .bind(&input.display_name)
                .bind(&input.location)
                .bind(revision)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                insert_audit_postgres(
                    &mut tx,
                    &actor,
                    now,
                    "device_profile.update",
                    device_ref,
                    json!({"display_name":input.display_name,"location":input.location,
                        "revision":revision}),
                )
                .await?;
                tx.commit().await?;
                Ok(DeviceProfile {
                    device_ref: device_ref.into(),
                    display_name: input.display_name,
                    location: input.location,
                    revision,
                    updated_at: now,
                })
            }
        }
    }

    pub async fn update_signal_profile(
        &self,
        actor: AuditActor,
        signal_ref: &str,
        input: SignalProfileInput,
        expected_revision: Option<i64>,
        now: i64,
    ) -> Result<SignalProfile, StorageError> {
        match self.inner.as_ref() {
            StorageInner::Sqlite { pool, .. } => {
                let mut tx = pool.begin().await?;
                let identity: Option<(String, String)> = sqlx::query_as(
                    "SELECT edge_node_id,series_key FROM inventory_signals WHERE signal_ref=?",
                )
                .bind(signal_ref)
                .fetch_optional(&mut *tx)
                .await?;
                let (edge_node_id, series_key) = identity.ok_or(StorageError::ProfileNotFound)?;
                let current: Option<i64> = sqlx::query_scalar(
                    "SELECT revision FROM signal_profiles WHERE edge_node_id=? AND series_key=?",
                )
                .bind(&edge_node_id)
                .bind(&series_key)
                .fetch_optional(&mut *tx)
                .await?;
                check_revision(current, expected_revision)?;
                let revision = current.unwrap_or(0) + 1;
                sqlx::query(
                    "INSERT INTO signal_profiles(edge_node_id,series_key,display_name,\
                     display_sensor_type,display_sensor_type_label,display_value_kind,\
                     display_unit_mode,display_unit,decimal_places,revision,updated_at) \
                     VALUES(?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(edge_node_id,series_key) DO UPDATE \
                     SET display_name=excluded.display_name,\
                     display_sensor_type=excluded.display_sensor_type,\
                     display_sensor_type_label=excluded.display_sensor_type_label,\
                     display_value_kind=excluded.display_value_kind,\
                     display_unit_mode=excluded.display_unit_mode,\
                     display_unit=excluded.display_unit,decimal_places=excluded.decimal_places,\
                     revision=excluded.revision,updated_at=excluded.updated_at",
                )
                .bind(&edge_node_id)
                .bind(&series_key)
                .bind(&input.display_name)
                .bind(&input.display_sensor_type)
                .bind(&input.display_sensor_type_label)
                .bind(&input.display_value_kind)
                .bind(&input.display_unit_mode)
                .bind(&input.display_unit)
                .bind(input.decimal_places)
                .bind(revision)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                insert_audit_sqlite(
                    &mut tx,
                    &actor,
                    now,
                    "signal_profile.update",
                    signal_ref,
                    signal_summary(&input, revision),
                )
                .await?;
                tx.commit().await?;
                Ok(signal_profile(signal_ref, input, revision, now))
            }
            StorageInner::Postgres { pool, .. } => {
                let mut tx = pool.begin().await?;
                let identity: Option<(String, String)> = sqlx::query_as(
                    "SELECT edge_node_id,series_key FROM inventory_signals \
                     WHERE signal_ref=$1 FOR UPDATE",
                )
                .bind(signal_ref)
                .fetch_optional(&mut *tx)
                .await?;
                let (edge_node_id, series_key) = identity.ok_or(StorageError::ProfileNotFound)?;
                let current: Option<i64> = sqlx::query_scalar(
                    "SELECT revision FROM signal_profiles WHERE edge_node_id=$1 AND series_key=$2",
                )
                .bind(&edge_node_id)
                .bind(&series_key)
                .fetch_optional(&mut *tx)
                .await?;
                check_revision(current, expected_revision)?;
                let revision = current.unwrap_or(0) + 1;
                sqlx::query(
                    "INSERT INTO signal_profiles(edge_node_id,series_key,display_name,\
                     display_sensor_type,display_sensor_type_label,display_value_kind,\
                     display_unit_mode,display_unit,decimal_places,revision,updated_at) \
                     VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) \
                     ON CONFLICT(edge_node_id,series_key) DO UPDATE SET \
                     display_name=excluded.display_name,\
                     display_sensor_type=excluded.display_sensor_type,\
                     display_sensor_type_label=excluded.display_sensor_type_label,\
                     display_value_kind=excluded.display_value_kind,\
                     display_unit_mode=excluded.display_unit_mode,\
                     display_unit=excluded.display_unit,decimal_places=excluded.decimal_places,\
                     revision=excluded.revision,updated_at=excluded.updated_at",
                )
                .bind(&edge_node_id)
                .bind(&series_key)
                .bind(&input.display_name)
                .bind(&input.display_sensor_type)
                .bind(&input.display_sensor_type_label)
                .bind(&input.display_value_kind)
                .bind(&input.display_unit_mode)
                .bind(&input.display_unit)
                .bind(input.decimal_places)
                .bind(revision)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                insert_audit_postgres(
                    &mut tx,
                    &actor,
                    now,
                    "signal_profile.update",
                    signal_ref,
                    signal_summary(&input, revision),
                )
                .await?;
                tx.commit().await?;
                Ok(signal_profile(signal_ref, input, revision, now))
            }
        }
    }
}

async fn postgres_diagnostic_signal_receipts(
    pool: &sqlx::PgPool,
    identities: &[(String, String, String)],
) -> Result<Vec<Option<i64>>, sqlx::Error> {
    if identities.is_empty() {
        return Ok(Vec::new());
    }

    // A correlated outer inventory variable makes PostgreSQL favour the
    // received-at history index even for a single-key lookup.  Keep the
    // inventory selection bounded, then issue one batched query whose arms
    // use direct parameters; this retains the proven md5+full-key plan
    // without reintroducing 64 sequential database round trips.
    let mut query = QueryBuilder::<Postgres>::new("");
    for (index, (edge_node_id, ledger_epoch, series_key)) in identities.iter().enumerate() {
        if index > 0 {
            query.push(" UNION ALL ");
        }
        query
            .push("SELECT (SELECT raw.received_at FROM raw_records AS raw WHERE raw.edge_node_id=");
        query.push_bind(edge_node_id);
        query.push(" AND raw.ledger_epoch=");
        query.push_bind(ledger_epoch);
        query.push(" AND md5(raw.series_key)=md5(");
        query.push_bind(series_key);
        query.push(") AND raw.series_key=");
        query.push_bind(series_key);
        query.push(" ORDER BY raw.received_at DESC,raw.pub_seq DESC LIMIT 1) AS received_at");
    }
    query
        .build_query_scalar()
        // A fresh prepared statement retains PostgreSQL's custom plan for
        // these direct identities; a cached generic plan can otherwise prefer
        // the broad received-at index solely because it satisfies LIMIT 1.
        .persistent(false)
        .fetch_all(pool)
        .await
}

fn check_revision(current: Option<i64>, expected: Option<i64>) -> Result<(), StorageError> {
    if current.is_some() && current != expected {
        return Err(StorageError::RevisionMismatch);
    }
    if current.is_none() && expected.is_some() {
        return Err(StorageError::RevisionMismatch);
    }
    Ok(())
}

fn signal_summary(input: &SignalProfileInput, revision: i64) -> serde_json::Value {
    json!({
        "display_name":input.display_name,
        "display_sensor_type":input.display_sensor_type,
        "display_sensor_type_label":input.display_sensor_type_label,
        "display_value_kind":input.display_value_kind,
        "display_unit_mode":input.display_unit_mode,
        "display_unit":input.display_unit,
        "decimal_places":input.decimal_places,
        "revision":revision
    })
}

fn signal_profile(
    signal_ref: &str,
    input: SignalProfileInput,
    revision: i64,
    now: i64,
) -> SignalProfile {
    SignalProfile {
        signal_ref: signal_ref.into(),
        display_name: input.display_name,
        display_sensor_type: input.display_sensor_type,
        display_sensor_type_label: input.display_sensor_type_label,
        display_value_kind: input.display_value_kind,
        display_unit_mode: input.display_unit_mode,
        display_unit: input.display_unit,
        decimal_places: input.decimal_places,
        revision,
        updated_at: now,
    }
}

fn device_sqlite(row: SqliteRow) -> Result<InventoryDevice, StorageError> {
    device_row(&row)
}
fn device_postgres(row: PgRow) -> Result<InventoryDevice, StorageError> {
    device_row(&row)
}
fn signal_sqlite(row: SqliteRow) -> Result<InventorySignal, StorageError> {
    signal_row(&row)
}
fn signal_postgres(row: PgRow) -> Result<InventorySignal, StorageError> {
    signal_row(&row)
}

fn device_row<R: Row>(row: &R) -> Result<InventoryDevice, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    Option<i64>: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
{
    Ok(InventoryDevice {
        device_ref: row.try_get("device_ref")?,
        edge_node_id: row.try_get("edge_node_id")?,
        system_id: row.try_get("system_id")?,
        identifier: row.try_get("identifier")?,
        state: row.try_get("state")?,
        presence: row.try_get("presence")?,
        model_id: row.try_get("model_id")?,
        display_name: row.try_get("display_name")?,
        location: row.try_get("location")?,
        profile_revision: row.try_get("profile_revision")?,
    })
}

fn signal_row<R: Row>(row: &R) -> Result<InventorySignal, StorageError>
where
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    i32: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
    Option<i64>: for<'r> sqlx::Decode<'r, R::Database> + sqlx::Type<R::Database>,
{
    Ok(InventorySignal {
        signal_ref: row.try_get("signal_ref")?,
        device_ref: row.try_get("device_ref")?,
        edge_node_id: row.try_get("edge_node_id")?,
        series_key: row.try_get("series_key")?,
        system_id: row.try_get("system_id")?,
        measurement_key: row.try_get("measurement_key")?,
        variant: row.try_get("variant")?,
        unit: row.try_get("unit")?,
        value_type: row.try_get("value_type")?,
        presence: row.try_get("presence")?,
        display_name: row.try_get("display_name")?,
        display_sensor_type: row.try_get("display_sensor_type")?,
        display_sensor_type_label: row.try_get("display_sensor_type_label")?,
        display_value_kind: row.try_get("display_value_kind")?,
        display_unit_mode: row.try_get("display_unit_mode")?,
        display_unit: row.try_get("display_unit")?,
        decimal_places: row.try_get("decimal_places")?,
        profile_revision: row.try_get("profile_revision")?,
    })
}
