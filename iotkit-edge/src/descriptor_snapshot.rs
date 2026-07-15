use iotkit_core_ledger::{self as ledger, DeviceState};
use iotkit_core_publish::PublishError;
use iotkit_core_publish::descriptor::{
    DESCRIPTOR_SCHEMA_VERSION, DescriptorDevice, DescriptorSignal, DescriptorSnapshot,
};
use iotkit_core_registry::{Resolution, ValueType};
use rusqlite::Connection;

pub fn build_descriptor_snapshot(
    conn: &Connection,
    edge_node_id: &str,
) -> Result<DescriptorSnapshot, PublishError> {
    let identity = ledger::load_edge_identity(conn)
        .map_err(|error| PublishError::Ledger(error.to_string()))?;
    if identity.edge_node_id != edge_node_id {
        return Err(PublishError::Invalid(
            "descriptor edge_node_id does not match initialized identity".into(),
        ));
    }
    let revision = ledger::descriptor_revision(conn)
        .map_err(|error| PublishError::Ledger(error.to_string()))?;
    let rows = ledger::list_devices(conn, true)
        .map_err(|error| PublishError::Ledger(error.to_string()))?;
    let mut devices = Vec::with_capacity(rows.len());
    let mut signals = Vec::new();
    for device in rows {
        let system_id = device.system_id.to_text();
        devices.push(DescriptorDevice {
            system_id: system_id.clone(),
            identifier: device.presentation_identifier,
            state: match device.state {
                DeviceState::Quarantined => "quarantined",
                DeviceState::Active => "active",
                DeviceState::Retired => "retired",
            }
            .into(),
        });
        let series = ledger::list_series_for_device(conn, &device.system_id)
            .map_err(|error| PublishError::Ledger(error.to_string()))?;
        for series in series.into_iter().filter(|series| !series.quarantined) {
            let resolution = iotkit_core_registry::find_resolution(conn, &series.measurement_key)
                .map_err(|error| PublishError::Invalid(error.to_string()))?
                .ok_or_else(|| {
                    PublishError::Invalid(format!(
                        "descriptor series has no registry resolution: {}",
                        series.measurement_key
                    ))
                })?;
            let entry = match resolution {
                Resolution::Entry(entry)
                | Resolution::Alias {
                    canonical: entry, ..
                } => entry,
            };
            signals.push(DescriptorSignal {
                series_key: ledger::series_key_of(
                    &device.system_id,
                    &series.measurement_key,
                    series.channel_index,
                    &series.variant,
                ),
                system_id: system_id.clone(),
                measurement_key: series.measurement_key,
                channel_index: (series.channel_index != ledger::CHANNEL_NA)
                    .then_some(series.channel_index),
                variant: series.variant,
                unit: entry.unit_ucum,
                value_type: match entry.value_type {
                    ValueType::Float => "float",
                    ValueType::Int => "int",
                    ValueType::Bool => "bool",
                    ValueType::Record => "record",
                }
                .into(),
            });
        }
    }
    devices.sort_by(|left, right| left.system_id.cmp(&right.system_id));
    signals.sort_by(|left, right| left.series_key.cmp(&right.series_key));
    Ok(DescriptorSnapshot {
        schema_version: DESCRIPTOR_SCHEMA_VERSION,
        edge_node_id: identity.edge_node_id,
        ledger_epoch: identity.ledger_epoch,
        descriptor_revision: revision,
        complete: true,
        devices,
        signals,
    })
}
