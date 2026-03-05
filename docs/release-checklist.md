# Release Checklist

This checklist is for internal consumers adopting a new `luau-analyze` release.

## Pre-release

1. Verify policy output:
   - `cargo run -p lan -- check --print-policy --json`
   - Assert `strict_mode=true`, `solver="new"`, `exposes_batch_queue=false`.
2. Run full validation:
   - `cargo test --workspace`
   - `cargo run -p xtask -- smoke`
3. Run latency benchmark gate:
   - `./scripts/bench-latency.sh`

## Runtime guidance

1. Start with no timeout and set one only after measuring script sizes.
2. For interactive use, prefer per-check `CheckOptions` with:
   - `timeout` between `10ms` and `200ms` for editor feedback loops.
   - shared `CancellationToken` reset/cancel between rapid edits.
3. Treat `CheckResult.timed_out` and `CheckResult.cancelled` as explicit
   realtime outcomes in callers.

## API migration checks

1. Prefer `Checker::with_options` to set default module/timeout policy.
2. Use `check_with_options` for per-call label/timeout/cancellation overrides.
3. Use `add_definitions_with_name` when loading multiple definition files so
   failure messages identify exact source files.

## Rollout

1. Tag pre-release (`vX.Y.Z-rcN`) and publish internal build artifact.
2. Run smoke tests in at least one downstream integration repository.
3. Record measured check latency and any regressions.
4. Update `plans/proj.md` with rollout status and follow-up items.
