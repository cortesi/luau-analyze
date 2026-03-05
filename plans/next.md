# Realtime Strict Checker Plan

This plan extends `luau-analyze` from a working baseline to a production-grade,
realtime checker. The design is strict-only, queue-free, and follows the most up
to date Luau mechanism without exposing legacy solver toggles.

1. Stage One: Lock Policy and Upstream Mechanism

Confirm the current best Luau checker path and freeze policy decisions before
feature work.

1. [x] Verify current Luau guidance for solver mode at pinned tag and latest tag using
   `crates/luau-analyze/luau/Analysis/include/Luau/Frontend.h` and upstream release notes.
2. [x] Decide a single solver policy for this project and encode it as a hard rule in
   code and docs (no user option for old solver).
3. [x] Update `docs/compatibility.md` and `README.md` with explicit policy text:
   strict-only mode, no batch queue, no legacy solver option.
4. [x] Add regression tests that fail if behavior drifts from strict-only semantics.
5. [x] Validate and document current single-file limitation for realtime checks until
   multi-file support is intentionally added.

2. Stage Two: Strict Realtime API Surface

Define a minimal API that supports realtime checks while staying small and
predictable.

1. [x] Introduce `CheckerOptions` in `crates/luau-analyze/src/lib.rs` for realtime-safe
   controls only (for example timeout and cancellation hooks), excluding solver toggles.
2. [x] Add per-call `CheckOptions` for lightweight runtime controls without rebuilding
   checker state.
3. [x] Keep strict mode enforced in shim and Rust API; do not expose non-strict paths.
4. [x] Document thread and lifecycle guarantees (`Send`/`Sync`, reuse semantics,
   definitions persistence).
5. [x] Add optional logical module labels for checks and definition loads so diagnostics
   can report real origins instead of hardcoded placeholders.

3. Stage Three: Shim Realtime Controls

Implement realtime-focused controls at the C++ boundary without queue features.

1. [x] Extend `crates/luau-analyze/shim/analyze_shim.cpp` ABI to accept timeout and
   cancellation state for single-check execution.
2. [x] Wire `FrontendOptions`/token fields needed for realtime interruption and guard
   long-running checks.
3. [x] Keep check flow single-module and immediate (`check("main")`), and explicitly do
   not add `queueModuleCheck` support.
4. [x] Preserve deterministic diagnostic ordering and ownership guarantees after ABI
   changes.
5. [x] Replace hardcoded `\"@definitions\"` and `\"main\"` with caller-provided module
   labels (defaulting to stable values when omitted).

4. Stage Four: Rust FFI and Public API Wiring

Expose the new realtime controls in safe Rust while preserving current behavior.

1. [x] Update `crates/luau-analyze/src/ffi.rs` for new shim structs and function
   signatures.
2. [x] Implement safe wrappers in `crates/luau-analyze/src/lib.rs` with clear error types
   for timeout and cancellation outcomes.
3. [x] Maintain backward-compatible defaults (`Checker::new()` and `check(&str)`) by
   mapping to strict realtime defaults.
4. [x] Add unit tests for new options, error conversions, and default-policy behavior.
5. [x] Add an internal RAII guard for `LuauCheckResult` so native allocations are freed
   even if Rust panics while materializing diagnostics.

5. Stage Five: `lan` CLI Realtime Demo Expansion

Use `lan` as the primary demonstration and manual validation tool.

1. [x] Add `lan check` flags for timeout and strict policy display in
   `crates/lan/src/main.rs`.
2. [x] Add machine-readable output mode (`--json`) for integration with tooling.
3. [x] Ensure CLI never exposes legacy solver selection or batch queue commands.
4. [x] Add CLI tests or golden output checks for timeout, cancellation, and JSON output.
5. [x] Prefix definition-load errors with the source file path when multiple `-d` files
   are used.
6. [x] Make demo script discovery recursive so nested example folders are included.

6. Stage Six: Example Corpus and Realtime Validation

Grow examples to exercise strict realtime behavior and edge cases.

1. [x] Add new scripts in `examples/scripts` for cancellation-like stress,
   timeout-sensitive code, and strict type edge cases.
2. [x] Add definitions fixtures that cover larger API surfaces and error paths.
3. [x] Expand `crates/luau-analyze/tests/integration.rs` with scenario tests for
   timeout, cancellation, and strict-only enforcement.
4. [x] Keep `cargo run -p xtask -- smoke` as a required gate and add expected-output
   assertions where practical.
5. [x] Add tests that verify module/definition labels appear in diagnostics for both
   success and failure paths.

7. Stage Seven: CI, Performance, and Release Gates

Finalize operational confidence for realtime use.

1. [x] Extend `.github/workflows/ci.yml` with Linux and macOS jobs that run full tests,
   `xtask smoke`, and strict policy assertions.
2. [x] Add sanitizer or memory-check job for C++ boundary safety (ASAN/UBSAN where
   supported).
3. [x] Add lightweight latency benchmark scripts for representative script sizes and set
   baseline thresholds.
4. [x] Update `docs/luau-update-playbook.md` with solver-policy verification steps for
   every Luau bump.

8. Stage Eight: Migration and Rollout

Adopt the improved checker without destabilizing downstream users.

1. [x] Add migration notes for API additions and policy clarifications in `README.md`.
2. [x] Prepare a release checklist for internal consumers, including timeout defaults and
   operational guidance.
3. [x] Run smoke tests in a downstream integration crate and document pre-release
   tagging in the release checklist.
4. [x] Record measured outcomes and follow-up work in `plans/proj.md` status notes.

## Execution Notes (2026-03-05)

- Verified Luau tag posture with:
  - `git -C crates/luau-analyze/luau tag --sort=-v:refname | head -n 5`
  - `git ls-remote --tags --refs https://github.com/luau-lang/luau.git`
- Verified strict/new solver APIs in:
  - `crates/luau-analyze/luau/Analysis/include/Luau/Frontend.h`
- Validated implementation with:
  - `cargo test --workspace`
  - `cargo run -p xtask -- smoke`
  - `./scripts/bench-latency.sh`
  - temporary downstream crate smoke run (path dependency on `luau-analyze`)
