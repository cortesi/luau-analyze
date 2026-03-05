# luau-analyze

Status: **pre-alpha**.

`luau-analyze` provides in-process Luau static analysis from Rust using Luau's
`Analysis` frontend and a small C shim.

## What It Does

- Creates reusable checkers in Rust
- Loads host type definitions once
- Checks Luau source in strict mode
- Returns structured diagnostics (errors and warnings)
- Runs without spawning external CLI processes

## Workspace Layout

- `crates/luau-analyze`: library crate
- `crates/lan`: demo command-line utility
- `examples/`: definitions and Luau scripts used by `lan demo` and tests

## Quick Start

Build and test:

```bash
cargo test --workspace
```

Run the demo suite:

```bash
cargo run -p lan -- demo
```

Run one script:

```bash
cargo run -p lan -- check -d examples/definitions/api.d.luau examples/scripts/01_ok_builder.luau
```

## Luau Pin

- Submodule path: `crates/luau-analyze/luau`
- Pinned tag for v0.1 target: `0.710`

## Documentation

- [Luau update playbook](docs/luau-update-playbook.md)
- [Compatibility and versioning policy](docs/compatibility.md)
