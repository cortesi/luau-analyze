# Shared library isolation: status and remaining work

The architectural switch from static archives to a private `.dylib`/`.so` loaded
via `libloading` is correct and well-justified. The `include_bytes!` embedding
has been implemented and makes binaries fully self-contained.

## Current state

### Done

- **Shared library build** (`build.rs`): all Luau sources compiled into a single
  `.dylib`/`.so`/`.dll` with `-fvisibility=hidden` and `-fPIC`. Only the 12
  `LUAU_ANALYZE_EXPORT` shim functions are exported.
- **Dynamic loading** (`ffi.rs`): `libloading` dlopen/dlsym with `OnceLock`
  caching. Function pointers stored in an `Api` struct.
- **Embedded native library** (`ffi.rs`): library bytes baked into the binary
  via `include_bytes!`. At first use, materialized into
  `$TMPDIR/luau-analyze/<fnv1a64-hash>/` with atomic PID-based temp-file writes
  and race-safe rename. Binary is fully self-contained — no external `.dylib`
  to ship.
- **Tests**: `checker_coexists_with_mlua` (symbol isolation regression) and
  `checker_works_without_build_directory_native_library` (deployment regression)
  both pass.

### Remaining

#### 1. Remove unused `parallel` feature from `cc` (trivial)

`Cargo.toml` still declares:

```toml
cc = { version = "1.2", features = ["parallel"] }
```

The `parallel` feature accelerates `cc::Build::compile()`, but the new build.rs
never calls `compile()` — it only uses `get_compiler()` to extract the
configured tool. The feature pulls in the `jobserver` crate for no benefit.
Change to:

```toml
cc = "1.2"
```

#### 2. Clean up stale temp directories (small)

Each rebuild produces a new FNV-1a hash, so `$TMPDIR/luau-analyze/` accumulates
old hash directories over time. During materialization, after a successful write,
delete sibling directories under `$TMPDIR/luau-analyze/` that don't match the
current hash. This keeps the temp footprint to a single copy.

```rust
// after successful materialization in materialize_native_library()
fn cleanup_stale_materializations(keep_dir: &Path) {
    let parent = match keep_dir.parent() {
        Some(p) => p,
        None => return,
    };
    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten() {
            if entry.path() != keep_dir {
                drop(fs::remove_dir_all(entry.path()));
            }
        }
    }
}
```

Best-effort with errors silenced — stale dirs are harmless, just wasteful.

#### 3. Recover parallel/incremental compilation (optional, nice-to-have)

All ~50 Luau `.cpp` files are passed to a single compiler invocation. This
loses the parallel and incremental compilation the old `cc::Build::compile()`
provided.

**Recommended approach — two-stage build:** Use `cc::Build::compile()` to
produce a static archive (with `-fPIC` and `-fvisibility=hidden` already
configured), then link that archive into a shared library:

```rust
// Stage 1: cc handles parallel compilation into a static archive
build.compile("luau_analyze_all");

// Stage 2: link into shared library
let mut link = Command::new(compiler.path());
link.arg("-shared").arg("-o").arg(&native_library);
// on macOS: -dynamiclib instead of -shared
link.arg(&format!("-L{}", out_dir.display()));
link.arg("-lluau_analyze_all");
// link C++ stdlib
```

This re-introduces the Apple AR complexity that the rewrite removed. Only worth
it if build times become a pain point.

#### 4. Restore type safety at the FFI boundary (minor)

The old FFI used opaque enum types (`enum LuauChecker {}`,
`enum LuauCancellationToken {}`) to prevent swapping handle types at compile
time. The new code uses `*mut c_void` everywhere.

Contained to internal code (public API is safe), but cheap newtypes would
recover the guard:

```rust
#[repr(transparent)]
struct CheckerHandle(*mut c_void);

#[repr(transparent)]
struct TokenHandle(*mut c_void);
```

---

## Context: how mlua handles Luau symbols

mlua (via `luau0-src` 0.18.3) takes **no isolation steps at all**:

- Compiles Luau into plain static archives with `cc::Build::compile()`
- Does not set `-fvisibility=hidden`
- Uses direct `extern "C"` FFI — all Luau symbols land in the binary's global
  symbol table with default visibility

Isolation is entirely luau-analyze's responsibility. The current shared-library
approach handles it correctly: `dlopen` with `RTLD_LOCAL` (the `libloading`
default) keeps the loaded symbols out of the global namespace, and
`-fvisibility=hidden` ensures nothing leaks from the `.dylib`'s export table
except the 12 `LUAU_ANALYZE_EXPORT` shim functions.
