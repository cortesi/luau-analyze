# luau-analyze

Status: **pre-alpha**.

`luau-analyze` provides in-process Luau static analysis from Rust using Luau's
`Analysis` frontend and a small C shim.

## What It Does

- Creates reusable checkers in Rust
- Loads host type definitions once
- Checks Luau source in strict mode only
- Returns structured diagnostics (errors and warnings)
- Runs without spawning external CLI processes

## Realtime Policy

`luau-analyze` intentionally exposes one checker policy:

- Strict mode is always enabled
- Solver mode is always `new`
- Batch queue APIs are not exposed (single-check, realtime flow only)

Use `checker_policy()` in Rust or `lan check --print-policy [--json]` to assert
policy from tools/CI.

## Workspace Layout

- `crates/luau-analyze`: library crate
- `crates/lan`: command-line checker utility
- `examples/`: definitions and Luau scripts used by `xtask smoke` and tests

## Quick Start

Build and test:

```bash
cargo test --workspace
```

Run smoke checks:

```bash
cargo run -p xtask -- smoke
```

Run one script:

```bash
cargo run -p lan -- check -d examples/definitions/api.d.luau examples/scripts/01_ok_builder.luau
```

Print policy:

```bash
cargo run -p lan -- check --print-policy --json
```

## Luau Pin

- Submodule path: `crates/luau-analyze/luau`
- Pinned tag for v0.1 target: `0.710`

## Known Limitations

- Checks are single-file. Cross-file `require(...)` resolution is intentionally
  out of scope for now.
- Timeout and cancellation are per-check controls for realtime interruption.
- The crate does not expose legacy solver selection or queue-based workflows.

## Migration Notes

Recent additions for realtime control:

- `CheckerOptions` for checker-wide defaults (module label, timeout)
- `CheckOptions` for per-call timeout/module/cancellation token
- `CancellationToken` for external interruption
- `CheckResult::{timed_out,cancelled}` outcome flags
- `Checker::add_definitions_with_name` and labeled checks for clearer origins

Behavioral policy is fixed:

- No option exists to select Luau old solver.
- No option exists to use queued/batch module checking.

## Documentation

- [Luau update playbook](docs/luau-update-playbook.md)
- [Compatibility and versioning policy](docs/compatibility.md)
- [Release checklist](docs/release-checklist.md)
