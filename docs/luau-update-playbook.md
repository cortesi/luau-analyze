# Luau Update Playbook

This project vendors Luau through a git submodule at
`crates/luau-analyze/luau`.

## Update Steps

1. Fetch latest tags and choose the target Luau tag.

```bash
git -C crates/luau-analyze/luau fetch --tags
git -C crates/luau-analyze/luau checkout <tag>
```

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
cargo run -p lan -- demo
```

5. Review API compatibility:
- Validate C++ shim signatures against:
  - `Analysis/include/Luau/Frontend.h`
  - `Analysis/include/Luau/FileResolver.h`
  - `Analysis/include/Luau/ConfigResolver.h`
- Ensure diagnostics and definition-loading tests still pass.

6. Commit:
- Submodule pointer update
- Any required `build.rs` or shim API fixes
- Test/docs adjustments

## Failure Modes To Check

- Build breaks due changed include paths or transitive library dependencies
- `loadDefinitionFile` signature changes
- Diagnostic structures or error formatting changes
- Runtime crashes due ownership mismatch across FFI
