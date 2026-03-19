# luau-analyze

Status: **Active** / **v0.1.0** (Public Release).

`luau-analyze` provides in-process Luau static analysis from Rust using Luau's
`Analysis` frontend and a small C shim.

## What It Does

- Creates reusable checkers in Rust
- Loads host type definitions once
- Checks Luau source in strict mode only
- Returns structured diagnostics (errors and warnings)
- Runs without spawning external CLI processes

## Usage

Add `luau-analyze` to your `Cargo.toml`:

```toml
[dependencies]
luau-analyze = "0.1.0"
```

Then use it in your Rust code:

```rust
use luau_analyze::Checker;

fn main() {
    let mut checker = Checker::new().expect("checker should initialize");
    
    checker.add_definitions(r#"
        declare function print(message: string): ()
    "#).expect("definitions should load");

    let result = checker.check(r#"
        --!strict
        print("Hello, world!")
    "#).expect("check should complete");

    if result.is_ok() {
        println!("Type check passed!");
    } else {
        for error in result.errors() {
            println!("Error at {}:{}: {}", error.line, error.col, error.message);
        }
    }
}
```

## Build Prerequisites

To build `luau-analyze`, you must have:
- A C/C++ toolchain (e.g. `clang` or `gcc`) installed on your system.
- `cmake` installed for configuring the Luau C++ build.
- If you are building from a git checkout, you must initialize the Luau submodule: `git submodule update --init --recursive`. (This is automatically included when using the crate from crates.io).
- Currently supported platforms: macOS and Linux. Windows is currently unsupported.

## Troubleshooting

- **Missing Luau sources:** If your build fails with "missing Luau sources", ensure you have initialized git submodules.
- **Unsupported toolchain:** If the build fails during C++ compilation, ensure you are not on Windows/MSVC and that your C++ compiler supports C++17.
- **Crate vs Git differences:** When using the crates.io package, the necessary Luau source files are bundled. When working on a local checkout, you must use the git submodule.

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

## Third-Party Licenses

This project contains code from the [Luau](https://github.com/luau-lang/luau) programming language, which is licensed under the MIT License. Luau itself contains code from Lua 5.1, which is also licensed under the MIT License. Copies of these licenses can be found in the vendored submodule at `crates/luau-analyze/luau/LICENSE.txt` and `crates/luau-analyze/luau/lua_LICENSE.txt`.

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
