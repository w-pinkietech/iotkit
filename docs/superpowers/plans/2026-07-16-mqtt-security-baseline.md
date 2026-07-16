# MQTT Security Baseline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the current Edge–Broker–Site MQTT path fail closed with an explicit trust policy, a fixed Mosquitto patch release, finite Broker limits, and executable rejection tests.

**Architecture:** This is the first of three independent implementation slices. It hardens the existing reference deployment without yet introducing the split-host connection-profile installer. TLS trust selection lives in each MQTT client, while Broker limits and the fixed image live in deployment artifacts. A Docker integration test exercises the security boundary from outside Mosquitto instead of treating generated configuration text as proof.

**Tech Stack:** Rust 1.88 / `rumqttc`, Go 1.25 / Eclipse Paho, Eclipse Mosquitto 2.0.22, Docker Compose, Bash, OpenSSL

## Global Constraints

- Production MQTT uses server TLS, hostname verification, anonymous-disabled Mosquitto, and a unique static username/password per principal.
- Trust mode is exactly `system_roots` or `bundle_only`; `bundle_only` must not inherit OS roots.
- Plain MQTT remains available only through the existing explicit `allow_insecure` development/test switch.
- Credentials remain owner-only files and must not appear in argv, environment values, rendered Compose, logs, errors, or test diagnostics.
- Use `eclipse-mosquitto:2.0.22`; moving to another patch release is a reviewed dependency update, not an implicit tag pull.
- Do not add Edge/Site client certificates or mTLS in this slice.
- Run focused tests after each task. Run `scripts/verify.sh` only once, after all Rust changes and before review/PR.

## Scope Boundary and Follow-on Plans

This plan intentionally does not pretend to finish the whole commissioning design.

1. This plan: explicit trust, fixed Broker version, finite limits, negative security matrix.
2. Next plan: split-host Broker/Site/Edge connection profiles and the local `install -> test -> activate -> rollback/revoke` lifecycle.
3. Following plan: generic Mosquitto certificate lifecycle component (`lego`, external bundle install, atomic activation, SIGHUP/probe/rollback, Pebble, systemd timer).
4. Later UI plan: read-only Site-observed Broker certificate and connection/delivery status.

---

### Task 1: Make Site MQTT trust selection explicit

**Files:**
- Create: `iotkit-site/internal/mqttsite/tls.go`
- Create: `iotkit-site/internal/mqttsite/tls_test.go`
- Modify: `iotkit-site/cmd/iotkit-site/main.go:56-82,302-318`
- Modify: `iotkit-site/cmd/iotkit-site/main_test.go:24-53`
- Modify: `deploy/compose.site.yaml:41-55`
- Modify: `scripts/test-site-mqtt.sh`
- Modify: `scripts/test-site-resilience.sh`

**Interfaces:**
- Produces: `mqttsite.TrustMode`, constants `TrustSystemRoots` and `TrustBundleOnly`.
- Produces: `mqttsite.LoadTLSConfig(mode TrustMode, bundlePath string) (*tls.Config, error)`.
- Consumes later: Task 4 starts Site with `--trust-mode bundle_only --ca-file <path>`.

- [ ] **Step 1: Write failing trust-policy tests**

Create `iotkit-site/internal/mqttsite/tls_test.go` with tests for the complete matrix:

```go
package mqttsite

import (
	"crypto/rand"
	"crypto/rsa"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/pem"
	"math/big"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestLoadTLSConfigSystemRoots(t *testing.T) {
	config, err := LoadTLSConfig(TrustSystemRoots, "")
	if err != nil {
		t.Fatal(err)
	}
	if config.MinVersion != tls.VersionTLS12 || config.RootCAs != nil {
		t.Fatalf("config = %#v", config)
	}
}

func TestLoadTLSConfigBundleOnlyDoesNotInheritSystemRoots(t *testing.T) {
	bundle := filepath.Join(t.TempDir(), "ca.pem")
	if err := os.WriteFile(bundle, testRootCertificatePEM(t), 0o600); err != nil {
		t.Fatal(err)
	}
	config, err := LoadTLSConfig(TrustBundleOnly, bundle)
	if err != nil {
		t.Fatal(err)
	}
	if config.RootCAs == nil || len(config.RootCAs.Subjects()) != 1 {
		t.Fatalf("bundle-only subjects = %d, want 1", len(config.RootCAs.Subjects()))
	}
}

func testRootCertificatePEM(t *testing.T) []byte {
	t.Helper()
	key, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		t.Fatal(err)
	}
	now := time.Now()
	template := &x509.Certificate{
		SerialNumber:          big.NewInt(1),
		Subject:               pkix.Name{CommonName: "IoTKit unit-test root"},
		NotBefore:             now.Add(-time.Hour),
		NotAfter:              now.Add(time.Hour),
		IsCA:                  true,
		BasicConstraintsValid: true,
		KeyUsage:              x509.KeyUsageCertSign,
	}
	der, err := x509.CreateCertificate(rand.Reader, template, template, &key.PublicKey, key)
	if err != nil {
		t.Fatal(err)
	}
	return pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der})
}

func TestLoadTLSConfigRejectsInvalidCombinations(t *testing.T) {
	tests := []struct {
		name string
		mode TrustMode
		path string
	}{
		{name: "unknown mode", mode: "automatic"},
		{name: "system roots with bundle", mode: TrustSystemRoots, path: "ca.pem"},
		{name: "bundle without file", mode: TrustBundleOnly},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			if _, err := LoadTLSConfig(test.mode, test.path); err == nil {
				t.Fatal("invalid trust configuration was accepted")
			}
		})
	}
}
```

- [ ] **Step 2: Run the focused test and confirm the missing API failure**

Run:

```bash
cd iotkit-site
go test ./internal/mqttsite -run 'TestLoadTLSConfig' -count=1
```

Expected: compile failure because `TrustMode` and `LoadTLSConfig` do not exist.

- [ ] **Step 3: Implement the Site trust policy**

Create `iotkit-site/internal/mqttsite/tls.go`:

```go
package mqttsite

import (
	"crypto/tls"
	"crypto/x509"
	"errors"
	"fmt"
	"os"
)

type TrustMode string

const (
	TrustSystemRoots TrustMode = "system_roots"
	TrustBundleOnly TrustMode = "bundle_only"
)

func LoadTLSConfig(mode TrustMode, bundlePath string) (*tls.Config, error) {
	switch mode {
	case TrustSystemRoots:
		if bundlePath != "" {
			return nil, errors.New("system_roots trust mode does not accept a CA file")
		}
		return &tls.Config{MinVersion: tls.VersionTLS12}, nil
	case TrustBundleOnly:
		if bundlePath == "" {
			return nil, errors.New("bundle_only trust mode requires a CA file")
		}
		pem, err := os.ReadFile(bundlePath)
		if err != nil {
			return nil, fmt.Errorf("read MQTT CA bundle: %w", err)
		}
		roots := x509.NewCertPool()
		if !roots.AppendCertsFromPEM(pem) {
			return nil, errors.New("MQTT CA bundle contains no certificates")
		}
		return &tls.Config{MinVersion: tls.VersionTLS12, RootCAs: roots}, nil
	default:
		return nil, fmt.Errorf("unsupported MQTT trust mode %q", mode)
	}
}
```

In `runServe`, replace the implicit `--ca-file` behavior with:

```go
trustMode := flags.String("trust-mode", "", "MQTT TLS trust mode: system_roots or bundle_only")
caFile := flags.String("ca-file", "", "PEM CA bundle for bundle_only trust")
// ...
var tlsConfig *tls.Config
if !*allowInsecure {
	if *trustMode == "" {
		return errors.New("--trust-mode is required unless --allow-insecure is used")
	}
	var err error
	tlsConfig, err = mqttsite.LoadTLSConfig(mqttsite.TrustMode(*trustMode), *caFile)
	if err != nil {
		return err
	}
}
```

Delete `loadTLSConfig` from `cmd/iotkit-site/main.go` and its old tests. Add command tests proving missing mode, invalid mode, `system_roots` plus CA, and `bundle_only` without CA fail before opening the database or reading a password value into diagnostics.

- [ ] **Step 4: Make every production Site invocation explicit**

Add these two arguments in `deploy/compose.site.yaml` before `--ca-file`:

```yaml
      - --trust-mode
      - bundle_only
```

For deliberate plaintext calls in `scripts/test-site-mqtt.sh` and `scripts/test-site-resilience.sh`, keep `--allow-insecure` and do not supply a fake trust mode. For TLS calls, supply exactly `--trust-mode bundle_only --ca-file "$ca_file"`.

- [ ] **Step 5: Run focused Site tests**

Run:

```bash
cd iotkit-site
gofmt -w internal/mqttsite/tls.go internal/mqttsite/tls_test.go cmd/iotkit-site/main.go cmd/iotkit-site/main_test.go
go test ./internal/mqttsite ./cmd/iotkit-site -count=1
```

Expected: both packages pass.

- [ ] **Step 6: Commit the Site trust boundary**

```bash
git add iotkit-site/internal/mqttsite/tls.go iotkit-site/internal/mqttsite/tls_test.go \
  iotkit-site/cmd/iotkit-site/main.go iotkit-site/cmd/iotkit-site/main_test.go \
  deploy/compose.site.yaml scripts/test-site-mqtt.sh scripts/test-site-resilience.sh
git commit -m "fix: make Site MQTT trust policy explicit"
```

---

### Task 2: Make Edge MQTT trust selection explicit

**Files:**
- Modify: `iotkit-edge/src/config.rs:66-75,138-145,353-390,817-870`
- Modify: `iotkit-edge/src/mqtt_publish_task.rs:17-28,71-96,442-451`
- Modify: `scripts/bootstrap-site.sh:231-239`
- Modify: `scripts/test-site-bootstrap.sh:176-184,221-232`
- Modify: `scripts/test-site-mqtt.sh`
- Modify: `scripts/test-site-resilience.sh`

**Interfaces:**
- Produces: `MqttTrustMode::{SystemRoots, BundleOnly}`.
- Changes: `MqttExitConfig` gains `trust_mode: MqttTrustMode` while retaining `allow_insecure` only for explicit tests.
- Consumes: Task 1 vocabulary `system_roots` and `bundle_only` in generated/user-facing configuration.

- [ ] **Step 1: Add failing Rust configuration tests**

Add tests to `iotkit-edge/src/config.rs` covering:

```rust
#[test]
fn resolve_mqtt_exit_requires_explicit_trust_mode_for_tls() {
    let mut raw = raw_with_defaults();
    raw.exit.mqtt = Some(RawMqttExitConfig {
        enabled: Some(true),
        password_file: Some("/run/secrets/mqtt-password".into()),
        ..RawMqttExitConfig::default()
    });
    assert!(matches!(
        resolve(raw, ConfigSource::DefaultsOnly),
        Err(ConfigError::Validation(message)) if message.contains("trust_mode")
    ));
}

#[test]
fn resolve_mqtt_exit_rejects_ambiguous_trust_inputs() {
    for (trust_mode, ca_file) in [
        ("system_roots", Some("/etc/iotkit/ca.pem")),
        ("bundle_only", None),
        ("automatic", None),
    ] {
        let mut raw = raw_with_defaults();
        raw.exit.mqtt = Some(RawMqttExitConfig {
            enabled: Some(true),
            password_file: Some("/run/secrets/mqtt-password".into()),
            trust_mode: Some(trust_mode.into()),
            ca_file: ca_file.map(str::to_owned),
            ..RawMqttExitConfig::default()
        });
        assert!(resolve(raw, ConfigSource::DefaultsOnly).is_err());
    }
}

#[test]
fn resolve_mqtt_exit_accepts_bundle_only() {
    let mut raw = raw_with_defaults();
    raw.exit.mqtt = Some(RawMqttExitConfig {
        enabled: Some(true),
        host: Some("broker.factory.example".into()),
        password_file: Some("/run/secrets/mqtt-password".into()),
        trust_mode: Some("bundle_only".into()),
        ca_file: Some("/etc/iotkit/broker-ca.pem".into()),
        ..RawMqttExitConfig::default()
    });
    let config = resolve(raw, ConfigSource::DefaultsOnly).unwrap();
    assert_eq!(config.mqtt_exit.unwrap().trust_mode, MqttTrustMode::BundleOnly);
}
```

- [ ] **Step 2: Run the focused Rust test and confirm failure**

Run:

```bash
cargo test -p iotkit-edge config::tests::resolve_mqtt_exit -- --nocapture
```

Expected: compile failure because `trust_mode` and `MqttTrustMode` do not exist.

- [ ] **Step 3: Implement strict Edge trust validation**

Add the raw field and resolved enum:

```rust
pub struct RawMqttExitConfig {
    pub enabled: Option<bool>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub password_file: Option<String>,
    pub trust_mode: Option<String>,
    pub ca_file: Option<String>,
    pub allow_insecure: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MqttTrustMode {
    SystemRoots,
    BundleOnly,
}
```

Add `trust_mode: MqttTrustMode` to `MqttExitConfig`. In `resolve_mqtt_exit`, validate after `password_file` and before constructing the result:

```rust
let allow_insecure = raw.allow_insecure.unwrap_or(false);
let (trust_mode, ca_file) = if allow_insecure {
    if raw.trust_mode.is_some() || raw.ca_file.is_some() {
        return Err(ConfigError::Validation(
            "exit.mqtt.allow_insecure cannot be combined with trust_mode or ca_file".into(),
        ));
    }
    (MqttTrustMode::SystemRoots, None)
} else {
    match (raw.trust_mode.as_deref(), raw.ca_file) {
        (Some("system_roots"), None) => (MqttTrustMode::SystemRoots, None),
        (Some("system_roots"), Some(_)) => return Err(ConfigError::Validation(
            "exit.mqtt.ca_file is forbidden with system_roots".into(),
        )),
        (Some("bundle_only"), Some(path)) if !path.trim().is_empty() => {
            (MqttTrustMode::BundleOnly, Some(PathBuf::from(path)))
        }
        (Some("bundle_only"), _) => return Err(ConfigError::Validation(
            "exit.mqtt.ca_file is required with bundle_only".into(),
        )),
        (Some(other), _) => return Err(ConfigError::Validation(format!(
            "exit.mqtt.trust_mode must be system_roots or bundle_only, got {other:?}"
        ))),
        (None, _) => return Err(ConfigError::Validation(
            "exit.mqtt.trust_mode is required when TLS is enabled".into(),
        )),
    }
};
```

Construct `MqttExitConfig` with `trust_mode`, `ca_file`, and `allow_insecure` from those validated locals. Do not include password contents in any error.

- [ ] **Step 4: Bind transport construction to the validated enum**

Change `RuntimeConfig.ca` to `Option<Vec<u8>>`, but make `read_ca` reject impossible states defensively:

```rust
fn read_ca(config: &MqttExitConfig) -> Result<Option<Vec<u8>>, String> {
    match (&config.trust_mode, &config.ca_file) {
        (MqttTrustMode::SystemRoots, None) => Ok(None),
        (MqttTrustMode::BundleOnly, Some(path)) => std::fs::read(path)
            .map(Some)
            .map_err(|error| format!("failed to read MQTT CA file {}: {error}", path.display())),
        _ => Err("invalid resolved MQTT trust configuration".to_string()),
    }
}
```

Import `MqttTrustMode`. Keep transport selection exactly:

```rust
options.set_transport(if runtime.connection.allow_insecure {
    Transport::tcp()
} else if let Some(ca) = runtime.ca {
    Transport::tls(ca, None, None)
} else {
    Transport::tls_with_default_config()
});
```

The configured `host` remains the DNS name used for TLS server-name verification; do not add a hostname-verification bypass.

- [ ] **Step 5: Update every Edge configuration fixture**

For TLS fragments generated by `scripts/bootstrap-site.sh`, emit:

```toml
[exit.mqtt]
enabled = true
host = "$broker_host"
port = $broker_port
password_file = "/etc/iotkit/mqtt-password"
trust_mode = "bundle_only"
ca_file = "/etc/iotkit/broker-ca.pem"
```

For deliberate plaintext fixtures, keep `allow_insecure = true` and omit both `trust_mode` and `ca_file`. Update Rust struct literals and equality assertions to the same rule.

- [ ] **Step 6: Run focused Edge and bootstrap-generation checks**

Run:

```bash
cargo fmt --all -- --check
cargo test -p iotkit-edge config::tests -- --nocapture
bash -n scripts/bootstrap-site.sh scripts/test-site-bootstrap.sh \
  scripts/test-site-mqtt.sh scripts/test-site-resilience.sh
```

Expected: all commands exit 0, and generated production TOML contains `trust_mode = "bundle_only"`.

- [ ] **Step 7: Commit the Edge trust boundary**

```bash
git add iotkit-edge/src/config.rs iotkit-edge/src/mqtt_publish_task.rs \
  scripts/bootstrap-site.sh scripts/test-site-bootstrap.sh \
  scripts/test-site-mqtt.sh scripts/test-site-resilience.sh
git commit -m "fix: make Edge MQTT trust policy explicit"
```

---

### Task 3: Pin and bound the production Broker profile

**Files:**
- Create: `deploy/mosquitto-image.env`
- Modify: `deploy/compose.site.yaml:1-31`
- Modify: `scripts/bootstrap-site.sh:89-91,192-205,228-229,242-255`
- Modify: `scripts/test-site-bootstrap.sh:172-204`
- Modify: `scripts/test-site-mqtt.sh`
- Modify: `scripts/test-site-resilience.sh`
- Modify: `docs/architecture.md` in “Site anatomy — what runs where”

**Interfaces:**
- Produces: `deploy/mosquitto-image.env` with `IOTKIT_MOSQUITTO_IMAGE=eclipse-mosquitto:2.0.22`.
- Produces: generated `site.env` carrying that non-secret exact image reference into Compose.
- Produces: one production Mosquitto listener with finite protocol and process limits.

- [ ] **Step 1: Add failing generated-profile assertions**

In `scripts/test-site-bootstrap.sh`, assert:

```bash
grep -Fxq 'IOTKIT_MOSQUITTO_IMAGE=eclipse-mosquitto:2.0.22' "$output/site.env"
for setting in \
  'message_size_limit 1048576' \
  'max_packet_size 1114112' \
  'max_inflight_messages 20' \
  'max_queued_messages 1000' \
  'max_connections 128' \
  'memory_limit 268435456'; do
  grep -Fxq "$setting" "$output/mosquitto/mosquitto.conf" || {
    echo "missing Mosquitto limit: $setting" >&2
    exit 1
  }
done
```

Also assert the rendered Compose contains `eclipse-mosquitto:2.0.22`, `no-new-privileges:true`, `cap_drop: [ALL]`, `pids_limit: 128`, and `mem_limit: 268435456`.

- [ ] **Step 2: Run bootstrap test far enough to confirm the new assertions fail**

Run:

```bash
scripts/test-site-bootstrap.sh
```

Expected: FAIL at the first missing image/limit assertion. If Docker access is unavailable, run the generation half using the script’s existing generated output assertions and record that the live portion remains for Task 4; do not declare the task complete.

- [ ] **Step 3: Add the exact image source and use it everywhere**

Create `deploy/mosquitto-image.env`:

```dotenv
IOTKIT_MOSQUITTO_IMAGE=eclipse-mosquitto:2.0.22
```

At the top of each shell script that invokes the image:

```bash
# shellcheck disable=SC1091
source "$repo_root/deploy/mosquitto-image.env"
```

Replace every `eclipse-mosquitto:2.0` shell invocation with `"$IOTKIT_MOSQUITTO_IMAGE"`. Generate the same variable into `site.env`. In Compose use:

```yaml
    image: "${IOTKIT_MOSQUITTO_IMAGE:?}"
```

- [ ] **Step 4: Add finite Broker limits and container confinement**

Generate these exact Mosquitto settings after the TLS options:

```conf
message_size_limit 1048576
max_packet_size 1114112
max_inflight_messages 20
max_queued_messages 1000
max_connections 128
memory_limit 268435456
```

The 1 MiB message limit matches the descriptor contract; the packet limit leaves 64 KiB for MQTT framing and properties. Add to the Broker service:

```yaml
    mem_limit: 268435456
    pids_limit: 128
    cap_drop:
      - ALL
    security_opt:
      - no-new-privileges:true
```

Do not add a second listener. Network-source restriction remains a deployment firewall responsibility because the permitted subnet differs by factory.

- [ ] **Step 5: Document the patch-update rule without duplicating D10**

In `docs/architecture.md`, state that `deploy/mosquitto-image.env` is the single repository source for the verified Broker patch release, and that updating it requires the Task 4 security matrix plus the normal final verification. Keep certificate automation and client profile lifecycle out of this paragraph.

- [ ] **Step 6: Run generated-profile and Compose checks**

Run:

```bash
bash -n scripts/bootstrap-site.sh scripts/test-site-bootstrap.sh \
  scripts/test-site-mqtt.sh scripts/test-site-resilience.sh
scripts/test-site-bootstrap.sh
```

Expected: `Production Site bootstrap TLS slice: OK` and no floating `eclipse-mosquitto:2.0` references under `deploy/` or `scripts/`:

```bash
! rg -n 'eclipse-mosquitto:2\.0([^.]|$)' deploy scripts
```

- [ ] **Step 7: Commit the bounded Broker profile**

```bash
git add deploy/mosquitto-image.env deploy/compose.site.yaml scripts/bootstrap-site.sh \
  scripts/test-site-bootstrap.sh scripts/test-site-mqtt.sh scripts/test-site-resilience.sh \
  docs/architecture.md
git commit -m "fix: bound the production Mosquitto profile"
```

---

### Task 4: Add the MQTT rejection integration matrix

**Files:**
- Create: `scripts/test-mqtt-security.sh`
- Modify: `scripts/verify.sh`
- Modify: `docs/redesign/decisions/D10-exit-authentication.md` only to mark which MVP-gate checks are executable

**Interfaces:**
- Produces: `scripts/test-mqtt-security.sh`, which creates isolated CA/Broker/principal fixtures, runs MQTT v5 clients, and destroys all temporary state.
- Does not produce: reusable production credentials, a certificate issuer, or a remote management API.

- [ ] **Step 1: Create the isolated Docker fixture and assertion helpers**

`scripts/test-mqtt-security.sh` must:

1. create a `mktemp -d` directory under `/tmp`, set `umask 077`, and trap Compose teardown plus deletion;
2. generate one active test CA/server certificate, one unrelated CA, and one expired server certificate with OpenSSL;
3. create principals `edge-a`, `edge-b`, and `site` with random password files and hashed Mosquitto password database;
4. create exact Edge A/B and Site ACLs from D10;
5. start the fixed `eclipse-mosquitto:2.0.22` image on an ephemeral TLS host port with no 1883 mapping;
6. create one owner-only default client configuration per principal at `$scratch/clients/<label>/.config/mosquitto_pub` and `mosquitto_sub`; put `-u`, `-P`, `--cafile`, and `-V mqttv5` there so no credential is passed in argv or environment;
7. run every client container with `--user "$(id -u):$(id -g)"`, `-e HOME=/work/clients/<label>`, and the scratch directory mounted at `/work`;
8. capture each client’s stdout/stderr and Broker logs under the scratch directory, but print only the case label and exit class on failure.

Use these assertion helpers in the script:

```bash
expect_success() {
  local label=$1
  shift
  if ! "$@" >"$scratch/$label.stdout" 2>"$scratch/$label.stderr"; then
    echo "expected MQTT success: $label" >&2
    exit 1
  fi
}

expect_rejected() {
  local label=$1
  shift
  if "$@" >"$scratch/$label.stdout" 2>"$scratch/$label.stderr"; then
    echo "expected MQTT rejection: $label" >&2
    exit 1
  fi
}

mqtt_client() {
  local home=$1 command=$2
  shift 2
  docker run --rm --network host \
    --user "$(id -u):$(id -g)" \
    -e "HOME=/work/clients/$home" \
    -v "$scratch:/work:ro" \
    "$IOTKIT_MOSQUITTO_IMAGE" "$command" "$@"
}
```

For the expired-certificate case, start a second isolated Broker container using the expired leaf on another ephemeral TLS port; do not weaken the Go clock or TLS verifier.

- [ ] **Step 2: Encode the exact allowed and rejected cases**

Use `mosquitto_pub -h localhost -p "$tls_port" -q 1 -t <topic> -m '{}'` and `mosquitto_sub -h localhost -p "$tls_port" -W 3 -t <topic>` through `mqtt_client`. The matrix is:

| Case | Expected |
|---|---|
| Edge A publishes `iotkit/v1/edge-nodes/edge-a/records` | success |
| Site publishes `iotkit/v1/edge-nodes/edge-a/accepted-through` | success |
| anonymous CONNECT | rejected CONNACK |
| Edge A with wrong password | rejected CONNACK |
| Edge A publishes Edge B records | rejected MQTT v5 PUBACK/disconnect |
| Edge A subscribes Edge B accepted-through | rejected MQTT v5 SUBACK/disconnect |
| Site publishes Edge A records | rejected MQTT v5 PUBACK/disconnect |
| valid principal with unrelated CA | TLS rejection |
| valid principal connects to `127.0.0.1` while leaf covers only `localhost` | hostname rejection |
| valid principal connects to expired-leaf Broker | certificate-time rejection |
| plaintext client connects to the TLS port | handshake rejection |
| TCP connect to the adjacent unused port standing in for 1883 | connection refused |

For ACL cases, non-zero client exit is required. A shell `timeout` exit code (`124`) is a test failure rather than proof of rejection. Generate the expired leaf with an OpenSSL CA database and fixed dates `20200101000000Z` through `20200102000000Z`; never disable clock validation.

After the matrix, run:

```bash
docker logs "$broker_container" >"$scratch/broker.log" 2>&1
for secret_file in "$scratch"/passwords/*.txt; do
  secret=$(<"$secret_file")
  if rg -F "$secret" "$scratch"/*.stdout "$scratch"/*.stderr "$scratch/broker.log"; then
    echo "MQTT credential leaked into diagnostics" >&2
    exit 1
  fi
done
```

Inspect the running container and fail unless its configured image is exactly `eclipse-mosquitto:2.0.22`, only the chosen TLS host port is published, and the Mosquitto config contains one `listener 8883` plus no `listener 1883`.

- [ ] **Step 3: Run the matrix and fix only observed contract mismatches**

Run:

```bash
scripts/test-mqtt-security.sh
```

Expected final line: `MQTT security matrix: OK`. Every allowed case must succeed and every rejected case must receive an explicit TLS, CONNACK, PUBACK, SUBACK, disconnect, or connection-refused result.

- [ ] **Step 4: Wire the matrix into the verification entry without running it twice**

Parse exactly one optional flag at the top of `scripts/verify.sh`:

```bash
full=false
if [[ ${1:-} == "--full" ]]; then
  full=true
  shift
fi
(($# == 0)) || { echo "usage: scripts/verify.sh [--full]" >&2; exit 2; }
```

After the existing Clippy check, add `cd iotkit-site && go test ./... && cd ..`. At the end add:

```bash
if [[ "$full" == true ]]; then
  echo "== scripts/test-mqtt-security.sh =="
  scripts/test-mqtt-security.sh
fi
```

The default verifier remains Docker-free; `--full` invokes the security matrix once.

- [ ] **Step 5: Update the D10 evidence statement**

Add one short evidence paragraph under `MVP security gate`: identify `scripts/test-mqtt-security.sh` as executable coverage for anonymous, wrong password, namespace ACL, Site overreach, wrong CA/hostname, expired leaf, missing plaintext listener, and secret-log leakage. Do not mark profile lifecycle, revocation, firewall, disk monitoring, or certificate automation complete; those belong to follow-on plans.

- [ ] **Step 6: Run final verification exactly once**

Run:

```bash
scripts/verify.sh --full
```

Expected: formatting, layer checks, Rust workspace tests, Clippy `-D warnings`, Go tests, existing Site integration tests, and the MQTT security matrix all pass. Save the command and final summary as review evidence; do not rerun the complete suite unless code changes after this point.

- [ ] **Step 7: Review the complete slice**

Review against D10 and the global constraints. The review must explicitly inspect:

- `bundle_only` creates a fresh root pool on both clients;
- no production invocation can silently select trust;
- no credential moved into argv/env/log/error;
- the exact Mosquitto patch is used in generation, hashing, Compose, and tests;
- ACL rejection is observed explicitly rather than inferred from a timeout;
- plain MQTT remains test-only;
- follow-on lifecycle/certificate work is not falsely reported complete.

If review changes behavior, rerun the affected focused test and then `scripts/verify.sh --full` once more before completion.

- [ ] **Step 8: Commit the executable security gate**

```bash
git add scripts/test-mqtt-security.sh scripts/verify.sh \
  docs/redesign/decisions/D10-exit-authentication.md
git commit -m "test: enforce MQTT security baseline"
```

## Completion Criteria

- Site and Edge require an explicit production trust mode.
- `bundle_only` trusts only the supplied bundle and retains hostname verification.
- The reference Broker uses Mosquitto 2.0.22 with finite message, packet, connection, inflight, queue, memory, and process limits.
- Docker evidence proves allowed Edge/Site operations work and the defined negative matrix is rejected.
- Secrets remain file-based and absent from observable command/config/log surfaces.
- D10 accurately distinguishes completed baseline checks from the still-pending profile lifecycle, revocation, firewall/disk operations, and certificate automation.
