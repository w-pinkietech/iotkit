use iotkit_output_adapter_api::{OutputAdapter, ProfilePolicy};
use iotkit_output_adapter_generic_mqtt_json_v1::{GenericMqttJsonAdapter, GenericMqttJsonPolicy};
use iotkit_output_adapter_pinikiet_mqtt_v1::{PinikietMqttAdapter, PinikietProfilePolicy};

pub struct OutputAdapterRegistration {
    pub adapter: &'static dyn OutputAdapter,
    pub profile_policy: &'static dyn ProfilePolicy,
}

static GENERIC_ADAPTER: GenericMqttJsonAdapter = GenericMqttJsonAdapter;
static GENERIC_POLICY: GenericMqttJsonPolicy = GenericMqttJsonPolicy;
static PINIKIET_ADAPTER: PinikietMqttAdapter = PinikietMqttAdapter;
static PINIKIET_POLICY: PinikietProfilePolicy = PinikietProfilePolicy;

static REGISTRY: &[OutputAdapterRegistration] = &[
    OutputAdapterRegistration {
        adapter: &GENERIC_ADAPTER,
        profile_policy: &GENERIC_POLICY,
    },
    OutputAdapterRegistration {
        adapter: &PINIKIET_ADAPTER,
        profile_policy: &PINIKIET_POLICY,
    },
];

#[must_use]
pub fn registered_output_adapters() -> &'static [OutputAdapterRegistration] {
    REGISTRY
}
