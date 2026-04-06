# luau-analyze

![Discord](https://img.shields.io/discord/1381424110831145070?style=flat-square&logo=rust&link=https%3A%2F%2Fdiscord.gg%2FfHmRmuBDxF)
[![Crates.io](https://img.shields.io/crates/v/luau-analyze.svg)](https://crates.io/crates/luau-analyze)
[![Documentation](https://docs.rs/luau-analyze/badge.svg)](https://docs.rs/luau-analyze)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

In-process Luau type checking for Rust. Wraps the Luau `Analysis` frontend via
a C shim so you can load host definitions, check Luau source, and get
structured diagnostics without spawning an external process.

## Usage

```rust
use luau_analyze::Checker;

let mut checker = Checker::new().expect("checker");

checker.add_definitions(r#"
    declare function greet(name: string): string
"#).expect("definitions");

let result = checker.check(r#"
    --!strict
    local msg: string = greet("world")
"#).expect("check");

assert!(result.is_ok());
```

Checkers are reusable — load definitions once, then check many sources. Each
check returns diagnostics with location, severity, and message.

## Building

Requires a C++17 toolchain (`clang` or `gcc`). From a git checkout, initialize
the Luau submodule first:

```bash
git submodule update --init --recursive
```

The crates.io package bundles the Luau sources, so no submodule step is needed
when using the published crate.

Supported platforms: macOS and Linux.

## Design

- Strict mode only, new solver only
- Single-file checks (no cross-file `require` resolution)
- Per-check timeout and cancellation via `CancellationToken`
- No batch/queue workflows

## Luau

Submodule pinned to tag `0.710` at `crates/luau-analyze/luau`.

Luau is licensed under the MIT License. Lua 5.1 code within Luau is also MIT
licensed. See `crates/luau-analyze/luau/LICENSE.txt` and
`crates/luau-analyze/luau/lua_LICENSE.txt`.
