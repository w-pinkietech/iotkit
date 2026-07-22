use super::*;

impl ExposureSnapshot {
    pub(crate) fn new(
        interface: impl Into<String>,
        addresses: impl IntoIterator<Item = IpAddr>,
        internet_default_route: bool,
    ) -> Self {
        Self {
            interface: interface.into(),
            addresses: addresses.into_iter().map(normalize_ip).collect(),
            _internet_default_route: internet_default_route,
        }
    }

    pub(crate) fn interface(&self) -> &str {
        &self.interface
    }

    pub(crate) fn addresses(&self) -> &BTreeSet<IpAddr> {
        &self.addresses
    }
}

impl ValidatedListenerConfig {
    /// Loopback is deliberately available only to crate tests. Product configuration must name a
    /// real private-network interface and CIDR.
    pub(crate) fn new_for_test(
        config: ListenerConfig,
        exposure: &ExposureSnapshot,
    ) -> Result<Self, ListenerError> {
        Self::validate(config, exposure, true)
    }
}
