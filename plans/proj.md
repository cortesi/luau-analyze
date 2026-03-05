# luau-analyze-rs: Implementation Plan

## Objective

Build a Rust crate that provides in-process Luau static analysis in strict mode
with host-provided type definitions and structured diagnostics.

## Success Criteria

1. A consumer can create a checker, load definitions, check source text, and
   read diagnostics.
2. Type checking runs in-process, with no CLI subprocess and no required disk
   I/O for checks.
3. macOS and Linux CI pass for the pinned Luau tag.
4. Public API docs and integration tests cover core behavior and error paths.
5. The crate pins a Luau tag and includes an update playbook.

## Scope

### In scope

- Compile Luau Analysis-related C++ code into static libraries from `build.rs`.
- Expose a small C shim around Luau `Frontend` operations needed for batch
  checks.
- Provide a minimal safe Rust API (`Checker`, `CheckResult`, `Diagnostic`,
  `Severity`).
- Load host API types before checks and keep a reusable checker instance.
- Return errors and warnings with source ranges and messages.

### Out of scope

- Runtime script execution (handled by `mlua` or another runtime crate).
- LSP features (hover, completion, incremental editor diagnostics).
- Full module graph or package-manager features in v0.
- Windows support in the initial release.

## Fixed Decisions

1. Luau pinning policy: use the latest stable Luau tag available when
   implementation starts, then freeze that tag for v0.1.0.
2. Release strategy: target internal/private consumption first, then public
   crates.io release after 1-2 production integrations.
3. Diagnostics ordering: sort by `(line, col, severity, message)` before
   returning results.

## Planned Architecture

```text
Rust app
  -> luau-analyze (safe API)
     -> FFI layer (raw extern C)
        -> C++ shim (extern C bridge)
           -> Luau Analysis Frontend
```

### Planned crate layout

```text
luau-analyze/
  Cargo.toml
  build.rs
  luau/                      # git submodule, pinned tag
  shim/analyze_shim.cpp
  src/lib.rs
  src/ffi.rs
  tests/integration.rs
```

### API shape target

```rust
pub struct Checker { /* opaque */ }

impl Checker {
    pub fn new() -> Result<Self, Error>;
    pub fn add_definitions(&mut self, defs: &str) -> Result<(), Error>;
    pub fn check(&mut self, source: &str) -> CheckResult;
}
```

Design rule: keep the Rust API intentionally small in v0.1.0 and avoid exposing
upstream Luau internals.

## Implementation Phases

### Phase 0: Bootstrap Repository

Goal: create a compilable Rust crate skeleton with CI scaffolding and no Luau
integration yet.

Tasks:

- Create `Cargo.toml`, `src/lib.rs`, `tests/` skeleton.
- Add formatting and lint configuration (`rustfmt`, `clippy` policy).
- Add CI workflow with matrix placeholders for macOS and Linux.
- Add `README.md` with explicit status `pre-alpha`.

Exit checks:

- `cargo test` passes with placeholder tests.
- CI runs on both target OSes.

### Phase 1: Build System Spike (C++ only)

Goal: compile required Luau components from vendored sources via `build.rs`.

Tasks:

- Add Luau as git submodule at the chosen pinned tag.
- Parse `luau/Sources.cmake` to derive source lists, or codify fixed lists with
  validation checks.
- Compile required libraries with `cc` crate and C++17 settings.
- Validate include paths and static link order.
- Record exact compiler and linker flags in docs.

Exit checks:

- `cargo build` succeeds on macOS.
- `cargo build` succeeds on Linux (gcc or clang).
- Build script fails with clear error when submodule is missing.

Validation commands:

```bash
cargo clean
cargo build -vv
```

### Phase 2: Minimal Shim and End-to-End Type Error

Goal: prove one full check round-trip from Rust to C ABI to C++ and back to
Rust diagnostics.

Tasks:

- Implement `shim/analyze_shim.cpp` with checker lifecycle and one check
  entrypoint.
- Enforce strict mode through a config resolver.
- Return diagnostics with line, column, message, and severity.
- Define and test ownership rules for result buffers.

Exit checks:

- Integration test catches a known strict-mode type mismatch.
- No leaks in memory-safety checks available in CI.

### Phase 3: Definitions Loading and Reuse Semantics

Goal: support loading host definitions once, then checking many scripts.

Tasks:

- Implement `add_definitions` in the C shim and Rust wrapper.
- Add tests proving definitions affect checks.
- Add tests proving checker reuse across multiple `check()` calls.
- Validate semantics after frontend and module reset behavior.

Exit checks:

- Tests prove definitions persist as intended across checks.
- Invalid definitions produce actionable errors.

### Phase 4: API Hardening and Ergonomics

Goal: finalize the safe Rust-facing contract.

Tasks:

- Define stable Rust error types (`Error` enum and conversions).
- Guarantee deterministic diagnostic ordering.
- Add helper methods (`is_ok`, `errors`, `warnings`).
- Add rustdoc examples for realistic usage.

Exit checks:

- Public API builds under a strict docs policy.
- All examples compile in doctests.

### Phase 5: CI, Compatibility, and Release Prep

Goal: ensure repeatable builds and maintainability.

Tasks:

- Finalize CI matrix for macOS and Linux.
- Add submodule update checklist and compatibility notes.
- Add versioning policy.
- Smoke-test crate in at least one downstream integration repo.

Exit checks:

- CI is green on required targets.
- Documented process exists for Luau tag updates.

## Validation Strategy

### Unit tests (Rust)

- `CheckResult::is_ok` behavior.
- Severity filtering helpers.
- Error conversion and display formatting.

### Integration tests (FFI end-to-end)

- Basic strict-mode type mismatch.
- Definition loading success and failure.
- Multiple checks with one checker instance.
- Empty script and syntax error edge cases.

### Platform checks

- macOS latest stable toolchain.
- Linux latest stable toolchain.

### Non-functional checks

- Build reproducibility with pinned submodule commit.
- Memory safety checks at the C/C++ boundary.

## Execution Rules

1. Treat this file as a living implementation plan and keep it current as each
   phase lands.
2. Any upstream-behavior claim must be backed by a pinned source reference
   before coding.
3. Do not merge phase work without corresponding tests for that phase.
4. Keep public API additions additive and minimal until first stable release.

## Immediate Next Actions

1. Execute Phase 0 in a dedicated implementation change.
2. At Phase 1 kickoff, record the exact Luau tag selected by the pinning policy.
3. After Phase 0, update this plan with measured outcomes and any phase
   adjustments.

## Current Status (2026-03-05)

- Phase 0: completed
- Phase 1: completed (Luau submodule pinned to `0.710`)
- Phase 2: completed
- Phase 3: completed
- Phase 4: completed
- Phase 5: completed

## Realtime Follow-up Status (2026-03-05)

- Strict realtime policy is now hard-enforced (`solver=new`, strict-only,
  queue-free) in shim, API, CLI, and CI assertions.
- Realtime controls are wired end-to-end:
  - timeout
  - cancellation token
  - per-call module labels
  - labeled definitions loading
- Outcome coverage is expanded with explicit timeout/cancellation flags and
  CLI JSON integration tests.
- Example corpus now includes nested realtime/strict/module-limit scripts plus
  additional definition fixtures.
- Baseline latency measurement from `scripts/bench-latency.sh`:
  - `examples/scripts/01_ok_builder.luau`: avg `12.29ms`, p95 `23.26ms`
  - `examples/scripts/realtime/08_timeout_sensitive_types_ok.luau`: avg
    `9.46ms`, p95 `9.99ms`
  - `examples/scripts/strict/11_generic_result_ok.luau`: avg `9.29ms`,
    p95 `9.39ms`
