# Future candidate: declarative sensor-device provisioning

Status: **unsettled future candidate; not design authority and not part of Plan 6**
Recorded: 2026-07-12

## Problem

Installing several sensor devices on one gateway should not require operators to repeat an
unstructured sequence of registry, network, scope, flow-class, and credential steps for every
unit. Some devices can accept generated settings; others require a vendor UI or manual work. The
candidate therefore standardizes desired inventory, validation, and progress without promising one
universal device-writing mechanism.

This is distinct from the existing pre-distribution deliverable for provisioning 20–100 gateway
boxes/cards. The two may share secret-custody and artifact-generation machinery, but this candidate
concerns multiple sensor devices attached to or sending to one gateway.

## Settled boundaries that this candidate must preserve

- Every network sender keeps an individual credential; shared credentials and secrets baked into
  shared firmware/images remain forbidden.
- Device registration, subject scopes, flow classes, and capacity-debt acceptance remain typed,
  audited R14 operations with the existing human gates.
- Registering devices never implicitly enables an HTTP/MQTT listener. Listener enablement remains
  a separate construction-tier action.
- Gateway ownership remains established per box through local `gatewayctl`; this candidate creates
  no network ownership/bootstrap exception.
- Credential, Wi-Fi password, private-key, and other secret plaintext is never stored in the
  declarative inventory or Git, nor printed through logs, errors, Debug, or audit.
- IP and MAC addresses are network/inventory attributes, never authentication identities.
- Plan 6, including Task 4, is not expanded by this candidate.

## Candidate behavior

- Maintain a non-secret desired inventory for multiple sensor devices. Candidate fields include
  hardware ID, display label, transport, subject scopes, flow class, and, when useful, MAC address,
  hostname, desired IP, and DHCP-reservation export information.
- Provide a side-effect-free plan that reports additions, differences, duplicate hardware/network
  identities, subject conflicts, capacity impact, authorization requirements, and external-network
  work before apply.
- Require an explicit human-approved apply. Bind apply to the reviewed plan digest and authority
  generation so stale plans cannot authorize changed state or larger capacity debt.
- Generate device-specific settings or operator work instructions through an appropriate adapter:
  USB, BLE, temporary setup AP, SSH, vendor UI, or manual entry are candidates. Unsupported devices
  may use an authenticated adapter/proxy or be reported honestly as unsupported.
- Track desired versus reported progress with states such as not configured, applied, connection
  confirmed, and failed. External actions must be idempotent and restartable; a global rollback
  cannot be promised for USB, vendor UI, DHCP, or other independently authoritative systems.
- Treat router/DHCP configuration as an external authority. The gateway may validate or export
  desired reservations, but must not claim they are applied without evidence from that authority.
- When this candidate provisions a network sender, generate its credential through the approved
  apply path; the existing manual D11 `device add` journey remains supported. Hand the credential
  over through an approved per-sender custody channel, device-bound where the device supports it,
  without aggregate plaintext output. Lost-response recovery follows the existing revoke/reissue
  or abandon/reissue lifecycle.
- Capacity planning evaluates the complete prospective live-authority set, not only newly listed
  devices, and binds any debt approval to the exact batch plan.

## Explicit non-goals and undecided choices

- No promise that every device can be configured automatically.
- No unconditional apply during gateway startup.
- Removing an inventory entry does not retire a device or revoke its credential. Those remain
  separate, explicitly approved operations to prevent mass silence after an editing mistake.
- The inventory syntax, CLI names, bundle format, secret-store integration, and device adapters are
  undecided until a separate Design Ready process.
- Static device IPs are not required for push-based HTTP/MQTT devices. DHCP reservations may be
  useful; polling devices may need stable addresses.
- QR codes were only an example raised during discussion. They are not a requirement or an adopted
  delivery mechanism.
- MQTT, pairing windows, `provisioned_key`, and fleet enrollment remain outside Plan 6 and require
  their own approved scope.

## Adoption gate

Before implementation, run a separate Design Ready decision covering R7 inventory ownership,
R14/R15 authorization and audit, R19 credential custody, external network authority, batch failure
and restart semantics, secret delivery, supported device adapters, and operator usability. If
adopted, propagate the settled result to D11 and the responsibility ledger rather than treating
this candidate note as canon.
