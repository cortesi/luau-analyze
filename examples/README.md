# Luau Demo Scripts

These scripts exercise `luau-analyze` and `xtask smoke`.

- `definitions/api.d.luau`: baseline host API definitions used by `xtask smoke`
- `definitions/game_api.d.luau`: broader API surface fixture for advanced examples
- `definitions/invalid_api.d.luau`: intentionally broken syntax fixture
- `definitions/invalid_types_api.d.luau`: intentionally broken type fixture
- `scripts/**/*.luau`: smoke scripts with expected outcome headers
  (`-- expect: pass|fail`) discovered recursively by `xtask smoke`.

Run smoke checks:

```bash
cargo run -p xtask -- smoke
```

Run a single script:

```bash
cargo run -p lan -- check -d examples/definitions/api.d.luau examples/scripts/01_ok_builder.luau
```
