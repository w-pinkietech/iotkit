#!/usr/bin/env python3
"""IoTKit local trial profile launcher.

The public input is a small, versioned TOML file. Generated runtime files and
credentials live outside the repository and are never copied into that TOML.
"""

from __future__ import annotations

import argparse
import getpass
import hashlib
import ipaddress
import json
import os
import secrets
import shutil
import stat
import subprocess
import sys
import tomllib
from pathlib import Path
from typing import Any


MOSQUITTO_IMAGE = "eclipse-mosquitto:2.0.22"
TOP_LEVEL_KEYS = frozenset({"config_version", "profile", "trial"})
TRIAL_KEYS = frozenset(
    {
        "console_bind",
        "console_port",
        "broker_bind",
        "broker_port",
        "sample_interval_ms",
    }
)


class ConfigError(ValueError):
    pass


class TrialConfig:
    def __init__(
        self,
        *,
        console_bind: str,
        console_port: int,
        broker_bind: str,
        broker_port: int,
        sample_interval_ms: int,
    ) -> None:
        self.console_bind = console_bind
        self.console_port = console_port
        self.broker_bind = broker_bind
        self.broker_port = broker_port
        self.sample_interval_ms = sample_interval_ms

    def normalized(self) -> dict[str, Any]:
        return {
            "console_bind": self.console_bind,
            "console_port": self.console_port,
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
    console_bind = _loopback(trial.get("console_bind", "127.0.0.1"), "trial.console_bind")
    broker_bind = _loopback(trial.get("broker_bind", "127.0.0.1"), "trial.broker_bind")
    console_port = _exact_int(trial.get("console_port", 8080), "trial.console_port", 1024, 65535)
    broker_port = _exact_int(trial.get("broker_port", 18883), "trial.broker_port", 1024, 65535)
    if console_bind == broker_bind and console_port == broker_port:
        raise ConfigError("trial.console_port and trial.broker_port must differ")
    sample_interval_ms = _exact_int(
        trial.get("sample_interval_ms", 1000),
        "trial.sample_interval_ms",
        250,
        60_000,
    )
    return TrialConfig(
        console_bind=console_bind,
        console_port=console_port,
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
message_size_limit 1048576
max_packet_size 1114112
"""


def render_edge_node_config(config: TrialConfig, db_path: str, password_file: str) -> str:
    return f"""[edge_node]
id = "trial"
db_path = {json.dumps(db_path)}
retention_days = 7

[api]
enabled = false

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
    environment = state / "trial.env"
    runtime = {}
    runtime_file = state / "runtime.json"
    if read_runtime and runtime_file.is_file():
        try:
            runtime = json.loads(runtime_file.read_text(encoding="utf-8"))
        except json.JSONDecodeError as error:
            raise ConfigError(f"invalid trial runtime metadata: {error}") from error
    runtime_uid = os.getuid() if hasattr(os, "getuid") else 10001
    runtime_gid = os.getgid() if hasattr(os, "getgid") else 10001
    content = "\n".join(
        [
            f"COMPOSE_PROJECT_NAME={_compose_project(state)}",
            f"IOTKIT_TRIAL_STATE={state}",
            f"IOTKIT_TRIAL_UID={runtime_uid}",
            f"IOTKIT_TRIAL_GID={runtime_gid}",
            f"IOTKIT_TRIAL_CONSOLE_BIND={config.console_bind}",
            f"IOTKIT_TRIAL_CONSOLE_PORT={config.console_port}",
            f"IOTKIT_TRIAL_BROKER_BIND={config.broker_bind}",
            f"IOTKIT_TRIAL_BROKER_PORT={config.broker_port}",
            f"IOTKIT_MOSQUITTO_IMAGE={MOSQUITTO_IMAGE}",
            f"IOTKIT_TRIAL_EDGE_ID={runtime.get('edge_id', 'edge-00000000000000000000000000000000')}",
            f"IOTKIT_TRIAL_EDGE_NODE_ID={runtime.get('edge_node_id', 'edge-node-pending')}",
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


def _read_admin_password(path: Path | None) -> str:
    if path is not None:
        password = path.read_text(encoding="utf-8").rstrip("\r\n")
    else:
        first = getpass.getpass("試用管理者のパスワード（12文字以上）: ")
        second = getpass.getpass("確認のため、もう一度入力: ")
        if first != second:
            raise ConfigError("passwords did not match")
        password = first
    if not 12 <= len(password) <= 128:
        raise ConfigError("admin password must be between 12 and 128 characters")
    return password


def _initialize(repo: Path, state: Path, config: TrialConfig, admin_password: str) -> list[str]:
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
    for child in ("edge", "node", "mosquitto", "secrets"):
        (state / child).mkdir(mode=0o700)

    node_password = secrets.token_urlsafe(32)
    edge_password = secrets.token_urlsafe(32)
    _write_private(state / "secrets" / "node-mqtt-password", node_password + "\n")
    _write_private(state / "secrets" / "edge-mqtt-password", edge_password + "\n")
    _write_private(state / "secrets" / "admin-password", admin_password + "\n")
    _write_private(
        state / "node" / "iotkit.toml",
        render_edge_node_config(config, "/data/node.db", "/run/secrets/node-mqtt-password"),
    )
    compose = _compose_args(repo, state, config)
    _run(compose + ["build", "edge", "edge-node"])
    try:
        initialized = json.loads(
            _run(
                compose
                + [
                    "run",
                    "--rm",
                    "--no-deps",
                    "--entrypoint",
                    "iotkit-edge-nodectl",
                    "edge-node",
                    "--db",
                    "/data/node.db",
                    "init",
                ],
                capture=True,
            )
        )
    except json.JSONDecodeError as error:
        raise ConfigError(f"edge-node init returned non-JSON output: {error}") from error
    try:
        edge_node_id = initialized["edge_node_id"]
    except (TypeError, KeyError) as error:
        raise ConfigError("edge-node init response is missing edge_node_id") from error
    edge_id = f"edge-{secrets.token_hex(16)}"
    _write_private(state / "runtime.json", json.dumps({"edge_id": edge_id, "edge_node_id": edge_node_id}))
    compose = _compose_args(repo, state, config)

    acl = f"""user edge
topic read iotkit/v1/edge-nodes/+/records
topic read iotkit/v1/edge-nodes/+/status
topic read iotkit/v1/edge-nodes/+/descriptors
topic read iotkit/v1/edge-nodes/+/activation/result
topic read iotkit/v1/edge-nodes/+/recovery/result
topic read iotkit/v1/edge-nodes/+/recovery/completion-ack
topic write iotkit/v1/edge-nodes/+/accepted-through
topic write iotkit/v1/edge-nodes/+/activation/request
topic write iotkit/v1/edge-nodes/+/recovery/request
topic write iotkit/v1/edge-nodes/+/recovery/completion

user {edge_node_id}
topic write iotkit/v1/edge-nodes/{edge_node_id}/records
topic write iotkit/v1/edge-nodes/{edge_node_id}/status
topic write iotkit/v1/edge-nodes/{edge_node_id}/descriptors
topic write iotkit/v1/edge-nodes/{edge_node_id}/activation/result
topic write iotkit/v1/edge-nodes/{edge_node_id}/recovery/result
topic write iotkit/v1/edge-nodes/{edge_node_id}/recovery/completion-ack
topic read iotkit/v1/edge-nodes/{edge_node_id}/accepted-through
topic read iotkit/v1/edge-nodes/{edge_node_id}/activation/request
topic read iotkit/v1/edge-nodes/{edge_node_id}/recovery/request
topic read iotkit/v1/edge-nodes/{edge_node_id}/recovery/completion
"""
    _write_private(state / "mosquitto" / "acl", acl)
    _write_private(state / "mosquitto" / "passwords", f"edge:{edge_password}\n{edge_node_id}:{node_password}\n")
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
    _run(
        compose
        + [
            "run",
            "--rm",
            "--no-deps",
            "--volume",
            f"{state / 'secrets' / 'admin-password'}:/run/secrets/admin-password:ro",
            "--entrypoint",
            "iotkit-edge",
            "edge",
            "account",
            "bootstrap",
            "--db",
            "/data/edge.db",
            "--login-id",
            "admin",
            "--display-name",
            "Trial administrator",
            "--password-file",
            "/run/secrets/admin-password",
        ]
    )
    (state / "secrets" / "admin-password").unlink()
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


def _trial_config_from_marker(document: dict[str, Any]) -> TrialConfig:
    raw = document.get("config")
    if not isinstance(raw, dict):
        raise ConfigError("trial state marker is invalid")
    try:
        return TrialConfig(
            console_bind=_loopback(raw["console_bind"], "trial.console_bind"),
            console_port=_exact_int(raw["console_port"], "trial.console_port", 1024, 65535),
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


def command_up(repo: Path, state: Path, config: TrialConfig, password_file: Path | None) -> None:
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
            compose = _initialize(repo, state, config, _read_admin_password(password_file))
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
    print(f"IoTKit Console: http://{config.console_bind}:{config.console_port}")
    print("ログインID: admin")
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
    print(f"IoTKit Console: http://{effective.console_bind}:{effective.console_port}")


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
    up = subcommands.add_parser("up", help="試用環境を起動")
    up.add_argument("--admin-password-file", type=Path)
    subcommands.add_parser("validate", help="設定を検証")
    subcommands.add_parser("status", help="状態を表示")
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
            command_up(repo, state, config, args.admin_password_file)
        elif args.command == "status":
            command_status(repo, state, config)
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
