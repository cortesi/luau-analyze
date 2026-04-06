# Changelog

## Unreleased (since `v0.0.1`)

- Isolated Luau symbols by loading an embedded private shared library at runtime, avoiding symbol collisions with `mlua` and other Luau embedders while keeping deployment self-contained.
- Restored typed internal FFI handles for checker and cancellation-token state, tightening compile-time safety across the Rust/native boundary.
- Removed outdated Luau version pinning from the docs and release tooling.
