use crate::storage::{AuditActor, Storage, StorageError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceProfileInput {
    pub display_name: String,
    pub location: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalProfileInput {
    pub display_name: String,
    pub display_sensor_type: String,
    pub display_sensor_type_label: String,
    pub display_value_kind: String,
    pub display_unit_mode: String,
    pub display_unit: String,
    pub decimal_places: i32,
}

impl SignalProfileInput {
    #[must_use]
    pub fn numeric(sensor_type: &str, unit: &str, decimal_places: i32) -> Self {
        Self {
            display_name: "Sensor".into(),
            display_sensor_type: sensor_type.into(),
            display_sensor_type_label: String::new(),
            display_value_kind: "numeric".into(),
            display_unit_mode: if unit.is_empty() {
                "dimensionless"
            } else {
                "unit"
            }
            .into(),
            display_unit: unit.into(),
            decimal_places,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceProfile {
    pub device_ref: String,
    pub display_name: String,
    pub location: String,
    pub revision: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalProfile {
    pub signal_ref: String,
    pub display_name: String,
    pub display_sensor_type: String,
    pub display_sensor_type_label: String,
    pub display_value_kind: String,
    pub display_unit_mode: String,
    pub display_unit: String,
    pub decimal_places: i32,
    pub revision: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryDevice {
    pub device_ref: String,
    pub edge_node_id: String,
    pub system_id: String,
    pub identifier: String,
    pub state: String,
    pub presence: String,
    pub model_id: String,
    pub display_name: String,
    pub location: String,
    pub profile_revision: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventorySignal {
    pub signal_ref: String,
    pub device_ref: String,
    pub edge_node_id: String,
    pub series_key: String,
    pub system_id: String,
    pub measurement_key: String,
    pub variant: String,
    pub unit: String,
    pub value_type: String,
    pub presence: String,
    pub display_name: String,
    pub display_sensor_type: String,
    pub display_sensor_type_label: String,
    pub display_value_kind: String,
    pub display_unit_mode: String,
    pub display_unit: String,
    pub decimal_places: i32,
    pub profile_revision: Option<i64>,
}

#[derive(Clone)]
pub struct InventoryProfiles {
    storage: Storage,
}

impl InventoryProfiles {
    #[must_use]
    pub fn new(storage: Storage) -> Self {
        Self { storage }
    }

    pub async fn devices(&self) -> Result<Vec<InventoryDevice>, StorageError> {
        self.storage.inventory_devices().await
    }

    pub async fn signals(&self) -> Result<Vec<InventorySignal>, StorageError> {
        self.storage.inventory_signals().await
    }

    pub async fn update_device(
        &self,
        actor: AuditActor,
        device_ref: &str,
        input: DeviceProfileInput,
        expected_revision: Option<i64>,
        now: i64,
    ) -> Result<DeviceProfile, StorageError> {
        let input = validate_device(input)?;
        self.storage
            .update_device_profile(actor, device_ref, input, expected_revision, now)
            .await
    }

    pub async fn update_signal(
        &self,
        actor: AuditActor,
        signal_ref: &str,
        input: SignalProfileInput,
        expected_revision: Option<i64>,
        now: i64,
    ) -> Result<SignalProfile, StorageError> {
        let input = validate_signal(input)?;
        self.storage
            .update_signal_profile(actor, signal_ref, input, expected_revision, now)
            .await
    }
}

fn validate_text(value: String, max_chars: usize, field: &str) -> Result<String, StorageError> {
    let value = value.trim().to_owned();
    if value.is_empty() || value.chars().count() > max_chars || value.chars().any(char::is_control)
    {
        return Err(StorageError::InvalidProfile(format!("{field} is invalid")));
    }
    Ok(value)
}

fn validate_device(input: DeviceProfileInput) -> Result<DeviceProfileInput, StorageError> {
    Ok(DeviceProfileInput {
        display_name: validate_text(input.display_name, 128, "display_name")?,
        location: validate_text(input.location, 256, "location")?,
    })
}

fn validate_signal(input: SignalProfileInput) -> Result<SignalProfileInput, StorageError> {
    let display_name = validate_text(input.display_name, 128, "display_name")?;
    let allowed_types = [
        "thermocouple",
        "temperature",
        "contact",
        "illuminance",
        "distance",
        "voltage",
        "current",
        "pressure",
        "humidity",
        "acceleration",
        "custom",
    ];
    if !allowed_types.contains(&input.display_sensor_type.as_str()) {
        return Err(StorageError::InvalidProfile(
            "display_sensor_type is invalid".into(),
        ));
    }
    let display_sensor_type_label = if input.display_sensor_type == "custom" {
        validate_text(
            input.display_sensor_type_label,
            64,
            "display_sensor_type_label",
        )?
    } else {
        String::new()
    };
    if !matches!(input.display_value_kind.as_str(), "numeric" | "boolean")
        || !matches!(input.display_unit_mode.as_str(), "unit" | "dimensionless")
        || !(0..=6).contains(&input.decimal_places)
    {
        return Err(StorageError::InvalidProfile(
            "display value settings are invalid".into(),
        ));
    }
    let display_unit = if input.display_unit_mode == "unit" {
        validate_text(input.display_unit, 32, "display_unit")?
    } else {
        String::new()
    };
    if input.display_value_kind == "boolean"
        && (input.display_unit_mode != "dimensionless" || input.decimal_places != 0)
    {
        return Err(StorageError::InvalidProfile(
            "boolean display must be dimensionless".into(),
        ));
    }
    Ok(SignalProfileInput {
        display_name,
        display_sensor_type: input.display_sensor_type,
        display_sensor_type_label,
        display_value_kind: input.display_value_kind,
        display_unit_mode: input.display_unit_mode,
        display_unit,
        decimal_places: input.decimal_places,
    })
}
