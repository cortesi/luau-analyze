# Luau Demo Scripts

These scripts exercise `luau-analyze` and the `lan` demo binary.

- `definitions/api.d.luau`: host API definitions loaded before checks
- `definitions/invalid_api.d.luau`: intentionally broken definitions file
- `scripts/*.luau`: demo scripts with expected outcome headers (`-- expect: pass|fail`)

Run all demos:

```bash
cargo run -p lan -- demo
```

Run a single script:

```bash
cargo run -p lan -- check -d examples/definitions/api.d.luau examples/scripts/01_ok_builder.luau
```
