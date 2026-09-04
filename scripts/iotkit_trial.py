#!/usr/bin/env python3
"""IoTKit local trial profile launcher.

Runs the redesigned product on one PC: the Edge Node with the trial-sample
Input Adapter and three pipelines, a standard Mosquitto Broker, and
`mosquitto_sub` as the independent consumer (`trial watch`). The public input
is a small, versioned TOML file. Generated runtime files and credentials live
outside the repository and are never copied into that TOML.
"""

from __future__ import annotations

import argparse
import hashlib
import ipaddress
import json
import os
import secrets
import shutil
import stat
import subprocess
import sys
import time
import tomllib
from pathlib import Path
from typing import Any


MOSQUITTO_IMAGE = "eclipse-mosquitto:2.0.22"
TOP_LEVEL_KEYS = frozenset({"config_version", "profile", "trial"})
TRIAL_KEYS = frozenset({"broker_bind", "broker_port", "sample_interval_ms"})
# The edge-node-id of the trial device; also its MQTT username. Fixed so the
# topics in the runbook are stable: iotkit/v1/edge-node/trial/...
EDGE_NODE_ID = "trial"
# Read-only Broker account used by `trial watch`.
VIEWER_USER = "viewer"
PIPELINE_IMPORT_TIMEOUT_S = 90


class ConfigError(ValueError):
    pass


class TrialConfig:
    def __init__(
        self,
        *,
        broker_bind: str,
        broker_port: int,
        sample_interval_ms: int,
    ) -> None:
        self.broker_bind = broker_bind
        self.broker_port = broker_port
        self.sample_interval_ms = sample_interval_ms

    def normalized(self) -> dict[str, Any]:
        return {
            "broker_bind": self.broker_bind,
            "broker_port": self.broker_port,
            "sample_interval_ms": self.sample_interval_ms,
        }


def _exact_int(value: Any, name: str, minimum: int, maximum: int) -> int:
    if type(value) is not int or not minimum <= value <= maximum:
        raise ConfigError(f"{name} must be an integer between {minimum} and {maximum}")
    return value


def _loopback(value: Any, name: str) -> str:
    if not isinstance(value, str):
        raise ConfigError(f"{name} must be a loopback IP address")
    try:
        address = ipaddress.ip_address(value)
    except ValueError as error:
        raise ConfigError(f"{name} must be a loopback IP address") from error
    if address.version != 4 or not address.is_loopback:
        raise ConfigError(f"{name} must be an IPv4 loopback address")
    return value


def load_config(path: Path) -> TrialConfig:
    try:
        with path.open("rb") as stream:
            document = tomllib.load(stream)
    except FileNotFoundError as error:
        raise ConfigError(f"configuration file not found: {path}") from error
    except tomllib.TOMLDecodeError as error:
        raise ConfigError(f"invalid TOML: {error}") from error

    unknown = sorted(set(document) - TOP_LEVEL_KEYS)
    if unknown:
        raise ConfigError(f"unknown key: {unknown[0]}")
    version = document.get("config_version")
    if type(version) is not int or version != 1:
        raise ConfigError("config_version must be 1")
    if document.get("profile") != "trial":
        raise ConfigError('profile must be "trial"')

    trial = document.get("trial", {})
    if not isinstance(trial, dict):
        raise ConfigError("trial must be a TOML table")
    unknown_trial = sorted(set(trial) - TRIAL_KEYS)
    if unknown_trial:
        raise ConfigError(f"unknown trial key: {unknown_trial[0]}")
    broker_bind = _loopback(trial.get("broker_bind", "127.0.0.1"), "trial.broker_bind")
    broker_port = _exact_int(trial.get("broker_port", 18883), "trial.broker_port", 1024, 65535)
    sample_interval_ms = _exact_int(
        trial.get("sample_interval_ms", 1000),
        "trial.sample_interval_ms",
        250,
        60_000,
    )
    return TrialConfig(
        broker_bind=broker_bind,
        broker_port=broker_port,
        sample_interval_ms=sample_interval_ms,
    )


def render_mosquitto_config() -> str:
    return """listener 1883 0.0.0.0
allow_anonymous false
password_file /mosquitto/config/passwords
acl_file /mosquitto/config/acl
persistence false
"""


def render_mosquitto_acl() -> str:
    """The device publishes only under its own edge-node-id; the viewer reads."""
    return f"""user {EDGE_NODE_ID}
topic write iotkit/v1/edge-node/{EDGE_NODE_ID}/status
topic write iotkit/v1/edge-node/{EDGE_NODE_ID}/observation/+/+

user {VIEWER_USER}
topic read iotkit/v1/edge-node/+/status
topic read iotkit/v1/edge-node/+/observation/+/+
"""


def render_edge_node_config(config: TrialConfig, db_path: str, password_file: str) -> str:
    return f"""[edge_node]
id = "{EDGE_NODE_ID}"
db_path = {json.dumps(db_path)}
retention_days = 7

[api]
enabled = false

[status]
heartbeat_interval = "30s"

[pipelines]
export_path = "/data/pipelines.toml"

[adapters.instances.trial_sample]
type = "trial-sample"
enabled = true
config_schema_version = 1
source = "trial:sample"
poll_interval_ms = {config.sample_interval_ms}

[output.mqtt]
enabled = true
host = {json.dumps(config.broker_bind)}
port = {config.broker_port}
password_file = {json.dumps(password_file)}
allow_insecure = true
"""


def render_pipelines_config() -> str:
    """Three pipelines over the two trial-sample inputs: the illuminance
    triangle wave as a measurement, and the contact square wave as a state and
    as an accumulated count of its rising edges."""
    return """[[pipeline]]
id = "sample-illuminance"
kind = "measurement"
unit = "lx"
display_name = "試用 照度"

[pipeline.input]
adapter = "trial_sample"
measurement_key = "illuminance_lux"

[[pipeline]]
id = "sample-contact"
kind = "state"
display_name = "試用 接点状態"

[pipeline.input]
adapter = "trial_sample"
measurement_key = "contact_state"

[pipeline.detector]
mode = "high-active"
rise_threshold = 0.5
fall_threshold = 0.5

[[pipeline]]
id = "sample-cycles"
kind = "accumulated-count"
trigger = "on-transition"
display_name = "試用 サイクル数"

[pipeline.input]
adapter = "trial_sample"
measurement_key = "contact_state"

[pipeline.detector]
mode = "high-active"
rise_threshold = 0.5
fall_threshold = 0.5
"""


def _repo_root() -> Path:
    return Path(__file__).resolve().parent.parent


def _state_dir() -> Path:
    base = os.environ.get("XDG_DATA_HOME")
    return (Path(base) if base else Path.home() / ".local" / "share") / "iotkit" / "trial"


def _compose_project(state: Path) -> str:
    identity = hashlib.sha256(str(state.resolve()).encode()).hexdigest()[:12]
    return f"iotkit-trial-{identity}"


def _run(args: list[str], *, capture: bool = False, input_text: str | None = None) -> str:
    result = subprocess.run(
        args,
        check=True,
        text=True,
        input=input_text,
        stdout=subprocess.PIPE if capture else None,
    )
    return result.stdout if capture else ""


def _compose_args(
    repo: Path,
    state: Path,
    config: TrialConfig,
    *,
    read_runtime: bool = True,
) -> list[str]:
    del read_runtime  # kept for call-site compatibility; no runtime metadata remains
    environment = state / "trial.env"
    runtime_uid = os.getuid() if hasattr(os, "getuid") else 10001
    runtime_gid = os.getgid() if hasattr(os, "getgid") else 10001
    content = "\n".join(
        [
            f"COMPOSE_PROJECT_NAME={_compose_project(state)}",
            f"IOTKIT_TRIAL_STATE={state}",
            f"IOTKIT_TRIAL_UID={runtime_uid}",
            f"IOTKIT_TRIAL_GID={runtime_gid}",
            f"IOTKIT_TRIAL_BROKER_BIND={config.broker_bind}",
            f"IOTKIT_TRIAL_BROKER_PORT={config.broker_port}",
            f"IOTKIT_MOSQUITTO_IMAGE={MOSQUITTO_IMAGE}",
        ]
    )
    _write_private(environment, content + "\n")
    return [
        "docker",
        "compose",
        "--env-file",
        str(environment),
        "-f",
        str(repo / "deploy" / "compose.trial.yaml"),
    ]


def _write_private(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
            stream.write(content)
    finally:
        path.chmod(stat.S_IRUSR | stat.S_IWUSR)


def _require_tools() -> None:
    if shutil.which("docker") is None:
        raise ConfigError("Docker with Docker Compose is required")
    _run(["docker", "compose", "version"], capture=True)


def _initialize(repo: Path, state: Path, config: TrialConfig) -> list[str]:
    state.mkdir(parents=True, exist_ok=True, mode=0o700)
    if any(state.iterdir()):
        raise ConfigError(f"trial state already exists: {state}; use status or reset")
    _write_private(
        state / "initializing.json",
        json.dumps(
            {
                "format": 1,
                "profile": "trial-initializing",
                "project": _compose_project(state),
                "config": config.normalized(),
            },
            sort_keys=True,
        )
        + "\n",
    )
    for child in ("node", "mosquitto", "secrets"):
        (state / child).mkdir(mode=0o700)

    node_password = secrets.token_urlsafe(32)
    viewer_password = secrets.token_urlsafe(32)
    _write_private(state / "secrets" / "node-mqtt-password", node_password + "\n")
    _write_private(state / "secrets" / "viewer-mqtt-password", viewer_password + "\n")
    _write_private(
        state / "node" / "iotkit.toml",
        render_edge_node_config(config, "/data/node.db", "/run/secrets/node-mqtt-password"),
    )
    _write_private(state / "node" / "pipelines-trial.toml", render_pipelines_config())
    _write_private(state / "mosquitto" / "acl", render_mosquitto_acl())
    _write_private(
        state / "mosquitto" / "passwords",
        f"{EDGE_NODE_ID}:{node_password}\n{VIEWER_USER}:{viewer_password}\n",
    )
    _write_private(state / "mosquitto" / "mosquitto.conf", render_mosquitto_config())
    runtime_uid = os.getuid() if hasattr(os, "getuid") else 10001
    runtime_gid = os.getgid() if hasattr(os, "getgid") else 10001
    _run(
        [
            "docker",
            "run",
            "--rm",
            "--user",
            f"{runtime_uid}:{runtime_gid}",
            "-v",
            f"{state / 'mosquitto'}:/work",
            MOSQUITTO_IMAGE,
            "mosquitto_passwd",
            "-U",
            "/work/passwords",
        ]
    )
    compose = _compose_args(repo, state, config)
    _run(compose + ["build", "edge-node"])
    _write_private(
        state / "trial-state.json",
        json.dumps(
            {
                "format": 1,
                "profile": "trial",
                "project": _compose_project(state),
                "config": config.normalized(),
            },
            sort_keys=True,
        )
        + "\n",
    )
    (state / "initializing.json").unlink()
    return compose


def _import_pipelines_once(compose: list[str], state: Path) -> None:
    """Imports the three trial pipelines into the running node the first time
    it is up. The node writes its edge-node-id into the database at startup,
    which `nodectl pipeline import` needs for the topic prefix, so the import
    is retried until the node has started."""
    marker = state / "pipelines-imported.json"
    if marker.is_file():
        return
    command = compose + [
        "exec",
        "-T",
        "edge-node",
        "iotkit-edge-nodectl",
        "--db",
        "/data/node.db",
        "pipeline",
        "import",
        "--replace-all",
        "--export-path",
        "/data/pipelines.toml",
        "/run/iotkit/pipelines-trial.toml",
    ]
    deadline = time.monotonic() + PIPELINE_IMPORT_TIMEOUT_S
    while True:
        result = subprocess.run(command, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if result.returncode == 0:
            _write_private(marker, json.dumps({"format": 1, "imported": 3}) + "\n")
            return
        if time.monotonic() >= deadline:
            raise ConfigError(
                "pipeline import did not succeed while waiting for the node to start: "
                + result.stderr.strip()
            )
        time.sleep(1.0)


def _trial_config_from_marker(document: dict[str, Any]) -> TrialConfig:
    raw = document.get("config")
    if not isinstance(raw, dict):
        raise ConfigError("trial state marker is invalid")
    try:
        return TrialConfig(
            broker_bind=_loopback(raw["broker_bind"], "trial.broker_bind"),
            broker_port=_exact_int(raw["broker_port"], "trial.broker_port", 1024, 65535),
            sample_interval_ms=_exact_int(
                raw["sample_interval_ms"], "trial.sample_interval_ms", 250, 60_000
            ),
        )
    except KeyError as error:
        raise ConfigError("trial state marker is invalid") from error


def _validated_marker(
    state: Path,
    config: TrialConfig,
    *,
    require_config_match: bool = True,
) -> dict[str, Any]:
    expected = _state_dir()
    if state.is_symlink() or state.resolve() != expected.resolve():
        raise ConfigError(f"refusing an unexpected or symlinked state path: {state}")
    marker = state / "trial-state.json"
    if marker.is_symlink() or not marker.is_file():
        raise ConfigError(f"refusing unrecognized trial state: {state}")
    try:
        document = json.loads(marker.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ConfigError("trial state marker is invalid") from error
    expected_keys = {"format", "profile", "project", "config"}
    if (
        not isinstance(document, dict)
        or set(document) != expected_keys
        or document.get("format") != 1
        or document.get("profile") != "trial"
        or document.get("project") != _compose_project(state)
    ):
        raise ConfigError(f"refusing unrecognized trial state: {state}")
    if require_config_match and document.get("config") != config.normalized():
        raise ConfigError(
            "trial state does not match this configuration; "
            "run ./scripts/iotkit trial reset --confirm-trial-data-loss "
            "(uses stored trial state), then up with the desired configuration"
        )
    return document


def _is_recognized_incomplete_state(state: Path, config: TrialConfig) -> bool:
    del config  # cleanup identity is path/project, not the live trial table
    if state.is_symlink() or state.resolve() != _state_dir().resolve():
        return False
    marker = state / "initializing.json"
    if not marker.exists() and not any(state.iterdir()):
        return True
    if marker.is_symlink() or not marker.is_file():
        return False
    try:
        document = json.loads(marker.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return False
    if not isinstance(document, dict):
        return False
    return (
        document.get("format") == 1
        and document.get("profile") == "trial-initializing"
        and document.get("project") == _compose_project(state)
        and isinstance(document.get("config"), dict)
    )


def _remove_incomplete_state(repo: Path, state: Path, config: TrialConfig) -> None:
    cleanup = _compose_args(repo, state, config, read_runtime=False)
    subprocess.run(
        cleanup + ["down", "--volumes", "--remove-orphans"],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    shutil.rmtree(state)


def command_up(repo: Path, state: Path, config: TrialConfig) -> None:
    _require_tools()
    marker = state / "trial-state.json"
    if marker.exists():
        _validated_marker(state, config)
        compose = _compose_args(repo, state, config)
    else:
        if state.exists():
            if not _is_recognized_incomplete_state(state, config):
                raise ConfigError(f"refusing unrecognized trial state: {state}")
            _remove_incomplete_state(repo, state, config)
        state_was_absent = not state.exists()
        try:
            compose = _initialize(repo, state, config)
        except BaseException as error:
            if state_was_absent and state.exists():
                try:
                    _remove_incomplete_state(repo, state, config)
                except Exception as cleanup_error:
                    print(
                        f"iotkit trial: cleanup after failed init also failed: {cleanup_error}",
                        file=sys.stderr,
                    )
            raise error
    _run(compose + ["up", "--detach"])
    _import_pipelines_once(compose, state)
    print(f"MQTT Broker: {config.broker_bind}:{config.broker_port}（edge-node-id: {EDGE_NODE_ID}）")
    print("表示: ./scripts/iotkit trial watch")
    print("停止: ./scripts/iotkit trial down")
    print("初期化: ./scripts/iotkit trial reset --confirm-trial-data-loss")
    print("現場への導入は docs/product/ja/operations/installation-and-recovery.md を参照してください。")


def command_down(
    repo: Path,
    state: Path,
    config: TrialConfig,
    *,
    remove_volumes: bool = False,
) -> None:
    if not (state / "trial-state.json").exists():
        print("試用環境はまだ作成されていません。")
        return
    document = _validated_marker(state, config, require_config_match=False)
    effective = _trial_config_from_marker(document)
    command = _compose_args(repo, state, effective) + ["down", "--remove-orphans"]
    if remove_volumes:
        command.append("--volumes")
    _run(command)
    if remove_volumes:
        print("試用環境を停止しました。")
    else:
        print("試用環境を停止しました。データは保持されています。")


def command_status(repo: Path, state: Path, config: TrialConfig) -> None:
    if not (state / "trial-state.json").exists():
        print("試用環境はまだ作成されていません。")
        return
    document = _validated_marker(state, config, require_config_match=False)
    effective = _trial_config_from_marker(document)
    _run(_compose_args(repo, state, effective) + ["ps"])
    print(f"MQTT Broker: {effective.broker_bind}:{effective.broker_port}（edge-node-id: {EDGE_NODE_ID}）")
    print("表示: ./scripts/iotkit trial watch")


def command_watch(repo: Path, state: Path, config: TrialConfig) -> None:
    """Follows every Observation and status the trial device publishes, as an
    independent consumer would: mosquitto_sub inside the Broker container with
    the read-only viewer account. Retained values arrive first."""
    if not (state / "trial-state.json").exists():
        print("試用環境はまだ作成されていません。")
        return
    document = _validated_marker(state, config, require_config_match=False)
    effective = _trial_config_from_marker(document)
    password = (state / "secrets" / "viewer-mqtt-password").read_text(encoding="utf-8").rstrip("\r\n")
    print("Ctrl-C で終了します。列: topic / retained / payload")
    subprocess.run(
        _compose_args(repo, state, effective)
        + [
            "exec",
            "-T",
            "broker",
            "mosquitto_sub",
            "-u",
            VIEWER_USER,
            "-P",
            password,
            "-t",
            "iotkit/v1/edge-node/+/status",
            "-t",
            "iotkit/v1/edge-node/+/observation/+/+",
            "-F",
            "%t %r %p",
        ],
        check=False,
    )


def command_reset(repo: Path, state: Path, config: TrialConfig, confirmed: bool) -> None:
    if not confirmed:
        raise ConfigError("reset deletes trial data; repeat with --confirm-trial-data-loss")
    marker = state / "trial-state.json"
    if not state.exists() or (not marker.exists() and not any(state.iterdir())):
        print("試用環境はまだ作成されていません。")
        return
    if not marker.exists():
        if not _is_recognized_incomplete_state(state, config):
            raise ConfigError(f"refusing unrecognized trial state: {state}")
        _remove_incomplete_state(repo, state, config)
        print("試用環境のデータを削除しました。")
        return
    document = _validated_marker(state, config, require_config_match=False)
    effective = _trial_config_from_marker(document)
    command_down(repo, state, effective, remove_volumes=True)
    shutil.rmtree(state.resolve())
    print("試用環境のデータを削除しました。")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="IoTKit trial profile")
    parser.add_argument("--config", type=Path, default=Path("iotkit.toml"))
    subcommands = parser.add_subparsers(dest="command", required=True)
    subcommands.add_parser("up", help="試用環境を起動")
    subcommands.add_parser("validate", help="設定を検証")
    subcommands.add_parser("status", help="状態を表示")
    subcommands.add_parser("watch", help="Observationとstatusを表示（Ctrl-Cで終了）")
    subcommands.add_parser("down", help="停止（データは保持）")
    reset = subcommands.add_parser("reset", help="試用データを削除")
    reset.add_argument("--confirm-trial-data-loss", action="store_true")
    args = parser.parse_args(argv)

    try:
        config = load_config(args.config)
        repo = _repo_root()
        state = _state_dir()
        if args.command == "validate":
            print(f"{args.config}: OK (trial profile, loopback only)")
        elif args.command == "up":
            command_up(repo, state, config)
        elif args.command == "status":
            command_status(repo, state, config)
        elif args.command == "watch":
            command_watch(repo, state, config)
        elif args.command == "down":
            command_down(repo, state, config)
        elif args.command == "reset":
            command_reset(repo, state, config, args.confirm_trial_data_loss)
    except (ConfigError, OSError, json.JSONDecodeError, subprocess.CalledProcessError) as error:
        print(f"iotkit trial: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
