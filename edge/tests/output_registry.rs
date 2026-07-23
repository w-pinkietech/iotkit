use std::collections::BTreeSet;

use iotkit_edge::composition::registered_output_adapters;

#[test]
fn production_registry_contains_each_builtin_once() {
    let adapters = registered_output_adapters();
    let ids = adapters
        .iter()
        .map(|registration| registration.adapter.descriptor().id)
        .collect::<BTreeSet<_>>();

    assert_eq!(
        ids,
        BTreeSet::from(["iotkit.mqtt-json.v1", "pinikiet.mqtt.v1"])
    );
    assert_eq!(ids.len(), adapters.len());
}
