# Luau Update Playbook

This project vendors Luau through a git submodule at
`crates/luau-analyze/luau`.

## Update Steps

1. Fetch latest tags and choose the target Luau tag.

```bash
git -C crates/luau-analyze/luau fetch --tags
git -C crates/luau-analyze/luau checkout <tag>
```

Also verify latest upstream release before pin updates:

```bash
git ls-remote --tags https://github.com/luau-lang/luau.git | tail -n 20
```

And review release notes:

- https://github.com/luau-lang/luau/releases

2. Verify submodule status:

```bash
git submodule status --recursive
```

3. Rebuild from clean state:

```bash
cargo clean
cargo build -vv -p luau-analyze
```

4. Run full checks:

```bash
cargo test --workspace
cargo run -p xtask -- smoke
```

5. Review API compatibility and solver policy:
- Validate C++ shim signatures against:
  - `Analysis/include/Luau/Frontend.h`
  - `Analysis/include/Luau/FileResolver.h`
  - `Analysis/include/Luau/ConfigResolver.h`
- Confirm `Frontend::setLuauSolverMode` still supports `SolverMode::New` and
  keep the project policy locked to `new` only.
- Confirm `FrontendOptions` still provides `moduleTimeLimitSec` and
  `cancellationToken` for realtime interruption.
- Ensure diagnostics, timeout/cancellation outcomes, and definition-loading
  tests still pass.

6. Commit:
- Submodule pointer update
- Any required `build.rs` or shim API fixes
- Test/docs adjustments

## Failure Modes To Check

- Build breaks due changed include paths or transitive library dependencies
- `loadDefinitionFile` signature changes
- Diagnostic structures or error formatting changes
- Runtime crashes due ownership mismatch across FFI
- Upstream solver policy changes that invalidate strict/new-only assumptions
