# AGENTS.md

## Project Context

`iotkit-next` is an in-progress remake of the legacy `iotkit` system.
The current goal is to rebuild legacy behavior incrementally while improving the separation of `core`, `adapter`, and `driver` layers.
When making changes, prefer preserving legacy-compatible behavior unless the change is an explicit architectural improvement or a deliberate deviation.
