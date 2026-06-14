# Dev Container

This container is for normal IoTKit development and non-hardware tests.

It installs the native dependencies needed by the Rust workspace, including
`libudev-dev`, `pkg-config`, and build tools. It also includes
`mosquitto-clients` for MQTT smoke checks.

The container is intended to reuse the RTX host's development identity and
agent context. These host paths are mounted into the `vscode` user:

- `~/.ssh` for Git over SSH
- `~/.gitconfig` and `~/.config/git` for Git identity and defaults
- `~/.config/gh` for GitHub CLI authentication
- `~/.codex` for Codex authentication, settings, memories, and sessions
- `~/.claude` and `~/.claude.json` for Claude Code authentication and context
- `~/.npmrc` for npm registry settings

The image installs `gh`, Node.js 22, Codex CLI, and Claude Code so those
host-backed credentials can be used directly inside the container.

## Usage

Open the repository in a Dev Containers compatible editor and choose
`Reopen in Container`.

Inside the container:

```bash
gh auth status
codex --version
claude --version
cargo test --workspace -- --test-threads=1
cargo build -p iotkit-rpi-local
```

Real Raspberry Pi hardware tests should still be run on the `iotkit` node:

```bash
cargo test -p rpi-local-adapter --test integration -- --ignored
```
