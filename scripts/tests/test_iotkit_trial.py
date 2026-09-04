import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock


REPO_ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = REPO_ROOT / "scripts" / "iotkit_trial.py"
SPEC = importlib.util.spec_from_file_location("iotkit_trial", MODULE_PATH)
iotkit_trial = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(iotkit_trial)


class TrialConfigTests(unittest.TestCase):
    def load(self, body: str):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "iotkit.toml"
            path.write_text(body, encoding="utf-8")
            return iotkit_trial.load_config(path)

    def test_two_line_config_uses_safe_defaults(self):
        config = self.load('config_version = 1\nprofile = "trial"\n')

        self.assertEqual(config.broker_port, 18883)
        self.assertEqual(config.sample_interval_ms, 1000)
        self.assertEqual(config.broker_bind, "127.0.0.1")

    def test_optional_trial_settings_are_accepted(self):
        config = self.load(
            'config_version = 1\nprofile = "trial"\n'
            "[trial]\nbroker_port = 18884\n"
            "sample_interval_ms = 2500\n"
        )

        self.assertEqual(config.broker_port, 18884)
        self.assertEqual(config.sample_interval_ms, 2500)

    def test_unknown_top_level_key_is_rejected(self):
        with self.assertRaisesRegex(iotkit_trial.ConfigError, "unknown key"):
            self.load('config_version = 1\nprofile = "trial"\nmagic = true\n')

    def test_unknown_trial_key_is_rejected(self):
        with self.assertRaisesRegex(iotkit_trial.ConfigError, "unknown trial key"):
            self.load(
                'config_version = 1\nprofile = "trial"\n'
                "[trial]\npublic_host = \"0.0.0.0\"\n"
            )

    def test_unknown_version_and_profile_are_rejected(self):
        with self.assertRaisesRegex(iotkit_trial.ConfigError, "config_version"):
            self.load('config_version = 2\nprofile = "trial"\n')
        with self.assertRaisesRegex(iotkit_trial.ConfigError, "profile"):
            self.load('config_version = 1\nprofile = "field"\n')
        with self.assertRaisesRegex(iotkit_trial.ConfigError, "config_version"):
            self.load('config_version = true\nprofile = "trial"\n')

    def test_non_loopback_bind_is_rejected(self):
        with self.assertRaisesRegex(iotkit_trial.ConfigError, "loopback"):
            self.load(
                'config_version = 1\nprofile = "trial"\n'
                '[trial]\nbroker_bind = "0.0.0.0"\n'
            )

    def test_console_keys_of_the_central_profile_are_rejected(self):
        with self.assertRaisesRegex(iotkit_trial.ConfigError, "unknown trial key: console_port"):
            self.load(
                'config_version = 1\nprofile = "trial"\n'
                "[trial]\nconsole_port = 18080\n"
            )

    def test_ipv6_loopback_is_rejected(self):
        with self.assertRaisesRegex(iotkit_trial.ConfigError, "IPv4 loopback"):
            self.load(
                'config_version = 1\nprofile = "trial"\n'
                '[trial]\nbroker_bind = "::1"\n'
            )

    def test_generated_pipelines_and_acl_follow_the_output_adapter_contract(self):
        pipelines = iotkit_trial.render_pipelines_config()
        self.assertEqual(pipelines.count("[[pipeline]]"), 3)
        for kind in ("measurement", "state", "accumulated-count"):
            self.assertIn(f'kind = "{kind}"', pipelines)
        self.assertIn('adapter = "trial_sample"', pipelines)
        self.assertIn('measurement_key = "illuminance_lux"', pipelines)
        self.assertIn('measurement_key = "contact_state"', pipelines)

        acl = iotkit_trial.render_mosquitto_acl()
        self.assertIn("user trial\n", acl)
        self.assertIn("topic write iotkit/v1/edge-node/trial/status", acl)
        self.assertIn("topic write iotkit/v1/edge-node/trial/observation/+/+", acl)
        self.assertIn("user viewer\n", acl)
        self.assertIn("topic read iotkit/v1/edge-node/+/observation/+/+", acl)
        self.assertNotIn("edge-nodes", acl)

    def test_generated_node_config_uses_trial_adapter_and_plaintext_local_broker(self):
        config = self.load('config_version = 1\nprofile = "trial"\n')
        rendered = iotkit_trial.render_edge_node_config(
            config, "/data/node.db", "/run/secrets/node-mqtt-password"
        )

        self.assertIn('[edge_node]\nid = "trial"\n', rendered)
        self.assertIn("[output.mqtt]\nenabled = true\n", rendered)
        self.assertIn('[status]\nheartbeat_interval = "30s"\n', rendered)
        self.assertIn('export_path = "/data/pipelines.toml"', rendered)
        self.assertNotIn("[exit.mqtt]", rendered)
        self.assertIn('type = "trial-sample"', rendered)
        self.assertIn('source = "trial:sample"', rendered)
        self.assertIn("poll_interval_ms = 1000", rendered)
        self.assertIn("allow_insecure = true", rendered)
        self.assertIn('host = "127.0.0.1"', rendered)
        self.assertNotIn("password =", rendered)

    def test_trial_broker_runs_as_requesting_user_without_persistence(self):
        compose = (REPO_ROOT / "deploy" / "compose.trial.yaml").read_text(
            encoding="utf-8"
        )
        mosquitto = iotkit_trial.render_mosquitto_config()

        self.assertIn('user: "${IOTKIT_TRIAL_UID}:${IOTKIT_TRIAL_GID}"', compose)
        self.assertIn('entrypoint: ["mosquitto"]', compose)
        self.assertNotIn("broker-data", compose)
        self.assertIn("persistence false", mosquitto)
        self.assertIn("allow_anonymous false", mosquitto)
        self.assertIn("condition: service_healthy", compose)
        self.assertIn("dockerfile: edge-node/Dockerfile", compose)
        self.assertNotIn("iotkit-edge\n", compose)
        self.assertNotIn("CONSOLE", compose)
        self.assertIn("pipelines-trial.toml", compose)

    def test_state_marker_is_exact_and_rejects_configuration_drift(self):
        config = self.load('config_version = 1\nprofile = "trial"\n')
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / "iotkit" / "trial"
            state.mkdir(parents=True)
            marker = {
                "format": 1,
                "profile": "trial",
                "project": iotkit_trial._compose_project(state),
                "config": config.normalized(),
            }
            (state / "trial-state.json").write_text(
                json.dumps(marker), encoding="utf-8"
            )
            with mock.patch.object(iotkit_trial, "_state_dir", return_value=state):
                iotkit_trial._validated_marker(state, config)
                marker["config"]["broker_port"] = 19999
                (state / "trial-state.json").write_text(
                    json.dumps(marker), encoding="utf-8"
                )
                with self.assertRaisesRegex(
                    iotkit_trial.ConfigError,
                    r"does not match.*reset --confirm-trial-data-loss",
                ):
                    iotkit_trial._validated_marker(state, config)
                document = iotkit_trial._validated_marker(
                    state, config, require_config_match=False
                )
                self.assertEqual(document["config"]["broker_port"], 19999)

    def test_down_and_status_use_marker_config_when_toml_drifts(self):
        stored = self.load(
            'config_version = 1\nprofile = "trial"\n'
            "[trial]\nbroker_port = 18884\n"
        )
        drifted = self.load('config_version = 1\nprofile = "trial"\n')
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / "iotkit" / "trial"
            state.mkdir(parents=True)
            (state / "trial-state.json").write_text(
                json.dumps(
                    {
                        "format": 1,
                        "profile": "trial",
                        "project": iotkit_trial._compose_project(state),
                        "config": stored.normalized(),
                    }
                ),
                encoding="utf-8",
            )
            observed: list[list[str]] = []

            def capture(args, **_kwargs):
                observed.append(list(args))
                return ""

            with mock.patch.object(iotkit_trial, "_state_dir", return_value=state):
                with mock.patch.object(iotkit_trial, "_run", side_effect=capture):
                    iotkit_trial.command_down(REPO_ROOT, state, drifted)
                    iotkit_trial.command_status(REPO_ROOT, state, drifted)

            env_files = [
                Path(args[args.index("--env-file") + 1]).read_text(encoding="utf-8")
                for args in observed
            ]
            self.assertTrue(all("IOTKIT_TRIAL_BROKER_PORT=18884" in text for text in env_files))
            self.assertFalse(any("CONSOLE" in text for text in env_files))

    def test_incomplete_state_is_recoverable_after_toml_drift(self):
        stored = self.load(
            'config_version = 1\nprofile = "trial"\n'
            "[trial]\nbroker_port = 18884\n"
        )
        drifted = self.load('config_version = 1\nprofile = "trial"\n')
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / "iotkit" / "trial"
            state.mkdir(parents=True)
            (state / "initializing.json").write_text(
                json.dumps(
                    {
                        "format": 1,
                        "profile": "trial-initializing",
                        "project": iotkit_trial._compose_project(state),
                        "config": stored.normalized(),
                    }
                ),
                encoding="utf-8",
            )
            with mock.patch.object(iotkit_trial, "_state_dir", return_value=state):
                self.assertTrue(
                    iotkit_trial._is_recognized_incomplete_state(state, drifted)
                )
                with mock.patch("subprocess.run") as run:
                    run.return_value = subprocess.CompletedProcess([], 0)
                    iotkit_trial.command_reset(
                        REPO_ROOT, state, drifted, confirmed=True
                    )
            self.assertFalse(state.exists())

    def test_failed_init_cleanup_error_does_not_hide_original_error(self):
        config = self.load('config_version = 1\nprofile = "trial"\n')
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / "iotkit" / "trial"
            original = RuntimeError("edge-node init failed")
            cleanup = subprocess.CalledProcessError(1, ["docker", "compose", "down"])

            def initialize(*_args, **_kwargs):
                state.mkdir(parents=True)
                (state / "initializing.json").write_text("{}", encoding="utf-8")
                raise original

            with mock.patch.object(iotkit_trial, "_state_dir", return_value=state):
                with mock.patch.object(iotkit_trial, "_require_tools"):
                    with mock.patch.object(
                        iotkit_trial, "_initialize", side_effect=initialize
                    ):
                        with mock.patch.object(
                            iotkit_trial,
                            "_remove_incomplete_state",
                            side_effect=cleanup,
                        ):
                            with mock.patch("builtins.print"):
                                with self.assertRaises(RuntimeError) as raised:
                                    iotkit_trial.command_up(REPO_ROOT, state, config)
            self.assertIs(raised.exception, original)

    def test_corrupt_state_marker_is_rejected(self):
        config = self.load('config_version = 1\nprofile = "trial"\n')
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / "iotkit" / "trial"
            state.mkdir(parents=True)
            (state / "trial-state.json").write_text("{}", encoding="utf-8")
            with mock.patch.object(iotkit_trial, "_state_dir", return_value=state):
                with self.assertRaisesRegex(
                    iotkit_trial.ConfigError, "unrecognized trial state"
                ):
                    iotkit_trial._validated_marker(state, config)

    def test_compose_project_depends_on_state_not_repository(self):
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / "state"
            state.mkdir()
            project = iotkit_trial._compose_project(state)
            self.assertEqual(project, iotkit_trial._compose_project(state))
            self.assertTrue(project.startswith("iotkit-trial-"))

    def test_only_exact_incomplete_state_is_recoverable(self):
        config = self.load('config_version = 1\nprofile = "trial"\n')
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / "iotkit" / "trial"
            state.mkdir(parents=True)
            with mock.patch.object(iotkit_trial, "_state_dir", return_value=state):
                self.assertTrue(
                    iotkit_trial._is_recognized_incomplete_state(state, config)
                )
            marker = {
                "format": 1,
                "profile": "trial-initializing",
                "project": iotkit_trial._compose_project(state),
                "config": config.normalized(),
            }
            (state / "initializing.json").write_text(
                json.dumps(marker), encoding="utf-8"
            )
            with mock.patch.object(iotkit_trial, "_state_dir", return_value=state):
                self.assertTrue(
                    iotkit_trial._is_recognized_incomplete_state(state, config)
                )
                marker["project"] = "unrelated-compose-project"
                (state / "initializing.json").write_text(
                    json.dumps(marker), encoding="utf-8"
                )
                self.assertFalse(
                    iotkit_trial._is_recognized_incomplete_state(state, config)
                )

    def test_incomplete_cleanup_ignores_leftover_files(self):
        config = self.load('config_version = 1\nprofile = "trial"\n')
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / "iotkit" / "trial"
            state.mkdir(parents=True)
            (state / "leftover.json").write_text('{"partial":', encoding="utf-8")
            observed_environment = []

            def record_environment(*_args, **_kwargs):
                observed_environment.append(
                    (state / "trial.env").read_text(encoding="utf-8")
                )
                return subprocess.CompletedProcess([], 0)

            with mock.patch("subprocess.run", side_effect=record_environment):
                iotkit_trial._remove_incomplete_state(REPO_ROOT, state, config)

            self.assertFalse(state.exists())
            self.assertIn("IOTKIT_TRIAL_BROKER_PORT=18883", observed_environment[0])

    def test_incomplete_cleanup_keeps_state_when_compose_down_fails(self):
        config = self.load('config_version = 1\nprofile = "trial"\n')
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / "iotkit" / "trial"
            state.mkdir(parents=True)
            (state / "leftover.json").write_text('{"partial":', encoding="utf-8")
            failure = subprocess.CalledProcessError(1, ["docker", "compose", "down"])

            with mock.patch("subprocess.run", side_effect=failure):
                with self.assertRaises(subprocess.CalledProcessError):
                    iotkit_trial._remove_incomplete_state(REPO_ROOT, state, config)

            self.assertTrue(state.exists())
            self.assertTrue((state / "leftover.json").exists())

    def test_reset_requires_confirmation_before_state_checks(self):
        config = self.load('config_version = 1\nprofile = "trial"\n')
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / "iotkit" / "trial"
            with mock.patch.object(iotkit_trial, "_state_dir", return_value=state):
                with self.assertRaisesRegex(
                    iotkit_trial.ConfigError, "--confirm-trial-data-loss"
                ):
                    iotkit_trial.command_reset(REPO_ROOT, state, config, confirmed=False)

    def test_reset_reports_when_trial_state_is_absent(self):
        config = self.load('config_version = 1\nprofile = "trial"\n')
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / "iotkit" / "trial"
            with mock.patch.object(iotkit_trial, "_state_dir", return_value=state):
                with mock.patch("builtins.print") as printed:
                    iotkit_trial.command_reset(
                        REPO_ROOT, state, config, confirmed=True
                    )
            printed.assert_called_once_with("試用環境はまだ作成されていません。")

    def test_reset_removes_recognized_incomplete_state(self):
        config = self.load('config_version = 1\nprofile = "trial"\n')
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / "iotkit" / "trial"
            state.mkdir(parents=True)
            marker = {
                "format": 1,
                "profile": "trial-initializing",
                "project": iotkit_trial._compose_project(state),
                "config": config.normalized(),
            }
            (state / "initializing.json").write_text(
                json.dumps(marker), encoding="utf-8"
            )
            with mock.patch.object(iotkit_trial, "_state_dir", return_value=state):
                with mock.patch("subprocess.run") as run:
                    run.return_value = subprocess.CompletedProcess([], 0)
                    with mock.patch("builtins.print") as printed:
                        iotkit_trial.command_reset(
                            REPO_ROOT, state, config, confirmed=True
                        )
            self.assertFalse(state.exists())
            printed.assert_called_with("試用環境のデータを削除しました。")
            run.assert_called_once()

    def test_pipeline_import_retries_until_the_node_has_started_and_runs_once(self):
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory)
            attempts = []

            def run(args, **_kwargs):
                attempts.append(list(args))
                if len(attempts) < 3:
                    return subprocess.CompletedProcess(
                        args, 1, stdout="", stderr="edge-node-id is not recorded"
                    )
                return subprocess.CompletedProcess(args, 0, stdout="{}", stderr="")

            with mock.patch("subprocess.run", side_effect=run):
                with mock.patch("time.sleep"):
                    iotkit_trial._import_pipelines_once(["docker", "compose"], state)
                    iotkit_trial._import_pipelines_once(["docker", "compose"], state)

            self.assertEqual(len(attempts), 3, "the marker prevents a second import")
            self.assertIn("iotkit-edge-nodectl", attempts[0])
            self.assertIn("--replace-all", attempts[0])
            self.assertTrue((state / "pipelines-imported.json").exists())

    def test_watch_subscribes_with_the_read_only_viewer_account(self):
        config = self.load('config_version = 1\nprofile = "trial"\n')
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / "iotkit" / "trial"
            (state / "secrets").mkdir(parents=True)
            (state / "secrets" / "viewer-mqtt-password").write_text("pw\n", encoding="utf-8")
            (state / "trial-state.json").write_text(
                json.dumps(
                    {
                        "format": 1,
                        "profile": "trial",
                        "project": iotkit_trial._compose_project(state),
                        "config": config.normalized(),
                    }
                ),
                encoding="utf-8",
            )
            with mock.patch.object(iotkit_trial, "_state_dir", return_value=state):
                with mock.patch("subprocess.run") as run:
                    with mock.patch("builtins.print"):
                        iotkit_trial.command_watch(REPO_ROOT, state, config)
            args = run.call_args.args[0]
            self.assertIn("mosquitto_sub", args)
            self.assertEqual(args[args.index("-u") + 1], "viewer")
            self.assertEqual(args[args.index("-P") + 1], "pw")
            self.assertIn("iotkit/v1/edge-node/+/observation/+/+", args)


if __name__ == "__main__":
    unittest.main()
