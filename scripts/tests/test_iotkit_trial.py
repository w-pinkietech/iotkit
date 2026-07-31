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

        self.assertEqual(config.console_port, 8080)
        self.assertEqual(config.broker_port, 18883)
        self.assertEqual(config.sample_interval_ms, 1000)
        self.assertEqual(config.console_bind, "127.0.0.1")
        self.assertEqual(config.broker_bind, "127.0.0.1")

    def test_optional_trial_settings_are_accepted(self):
        config = self.load(
            'config_version = 1\nprofile = "trial"\n'
            "[trial]\nconsole_port = 18080\nbroker_port = 18884\n"
            "sample_interval_ms = 2500\n"
        )

        self.assertEqual(config.console_port, 18080)
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
                '[trial]\nconsole_bind = "0.0.0.0"\n'
            )

    def test_generated_node_config_uses_trial_adapter_and_plaintext_local_broker(self):
        config = self.load('config_version = 1\nprofile = "trial"\n')
        rendered = iotkit_trial.render_edge_node_config(
            config, "/data/node.db", "/run/secrets/node-mqtt-password"
        )

        self.assertIn('type = "trial-sample"', rendered)
        self.assertIn('source = "trial:sample"', rendered)
        self.assertIn("poll_interval_ms = 1000", rendered)
        self.assertIn("allow_insecure = true", rendered)
        self.assertIn('host = "127.0.0.1"', rendered)
        self.assertNotIn("password =", rendered)

    def test_trial_broker_reads_owner_only_files_as_the_requesting_user(self):
        compose = (REPO_ROOT / "deploy" / "compose.trial.yaml").read_text(
            encoding="utf-8"
        )
        broker_service = compose.split("\n  edge:\n", maxsplit=1)[0]

        self.assertIn('user: "${IOTKIT_TRIAL_UID}:${IOTKIT_TRIAL_GID}"', broker_service)
        self.assertIn('entrypoint: ["mosquitto"]', broker_service)
        self.assertNotIn("broker-data", compose)
        self.assertIn("persistence false", MODULE_PATH.read_text(encoding="utf-8"))

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
                with self.assertRaisesRegex(iotkit_trial.ConfigError, "does not match"):
                    iotkit_trial._validated_marker(state, config)

    def test_corrupt_state_marker_is_rejected(self):
        config = self.load('config_version = 1\nprofile = "trial"\n')
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / "iotkit" / "trial"
            state.mkdir(parents=True)
            (state / "trial-state.json").write_text("{}", encoding="utf-8")
            with mock.patch.object(iotkit_trial, "_state_dir", return_value=state):
                with self.assertRaisesRegex(iotkit_trial.ConfigError, "does not match"):
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

    def test_incomplete_cleanup_ignores_a_partial_runtime_file(self):
        config = self.load('config_version = 1\nprofile = "trial"\n')
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / "iotkit" / "trial"
            state.mkdir(parents=True)
            (state / "runtime.json").write_text('{"edge_id":', encoding="utf-8")
            observed_environment = []

            def record_environment(*_args, **_kwargs):
                observed_environment.append(
                    (state / "trial.env").read_text(encoding="utf-8")
                )
                return subprocess.CompletedProcess([], 0)

            with mock.patch("subprocess.run", side_effect=record_environment):
                iotkit_trial._remove_incomplete_state(REPO_ROOT, state, config)

            self.assertFalse(state.exists())
            self.assertIn(
                "IOTKIT_TRIAL_EDGE_NODE_ID=edge-node-pending",
                observed_environment[0],
            )

    def test_incomplete_cleanup_keeps_state_when_compose_down_fails(self):
        config = self.load('config_version = 1\nprofile = "trial"\n')
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / "iotkit" / "trial"
            state.mkdir(parents=True)
            (state / "runtime.json").write_text('{"edge_id":', encoding="utf-8")
            failure = subprocess.CalledProcessError(1, ["docker", "compose", "down"])

            with mock.patch("subprocess.run", side_effect=failure):
                with self.assertRaises(subprocess.CalledProcessError):
                    iotkit_trial._remove_incomplete_state(REPO_ROOT, state, config)

            self.assertTrue(state.exists())
            self.assertTrue((state / "runtime.json").exists())


if __name__ == "__main__":
    unittest.main()
