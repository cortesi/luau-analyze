# luau-analyze-rs: Design Document

## Problem Statement

We need an embeddable, statically typed, sandboxed scripting language for a Rust
application where scripts are primarily authored by LLMs. The scripting API is
complex (thousands of lines of typed Rust definitions including builder patterns,
enum unions, trait-based polymorphism, and Result/Option wrapping). Scripts may
be executed on a deferred basis — authored now, run an hour from now — making
pre-execution type checking essential for correctness.

After evaluating Rhai (no type system), TypeScript/Deno (excessive weight, poor
embedding story, V8 dependency), and Rust-compiled-to-WASM (toolchain weight,
compilation latency), Luau via mlua emerged as the strongest runtime choice:
best-in-class sandboxing (memory limits, CPU interruption, cooperative
yield/resume), decent LLM familiarity via the Roblox corpus, mature Rust
bindings, and a real gradual type system with `--!strict` mode.

The gap: Luau's type checker (`Luau.Analysis`) is a C++17 library with no C API
and no existing Rust bindings. The mlua crate binds the VM and Compiler (which
have clean C APIs) but not Analysis. The `luau-analyze` CLI tool wraps the
Analysis library but requires disk I/O and subprocess invocation. Nobody in the
Rust ecosystem has bridged this gap.

This project creates that bridge.

## Goals

1. **In-process Luau type checking from Rust.** No subprocess, no disk I/O, no
   external binaries. Source and type definitions are provided as strings;
   diagnostics are returned as structured Rust types.

2. **Strict mode checking against custom API definitions.** Users can load type
   definitions describing their host API (using Luau's `declare` syntax or
   module-based type stubs), then check scripts against those definitions. All
   checks run in `--!strict` mode.

3. **macOS and Linux builds.** The crate initially targets macOS (clang), with
   Linux (gcc/clang) as a near-term second platform. Windows is out of scope
   for now.

4. **Minimal API surface.** The Rust API exposes exactly what's needed for batch
   type checking: create a checker, load definitions, check source, get
   diagnostics. No LSP, no autocomplete, no incremental checking.

5. **Luau version tracking.** Luau sources are vendored via git submodule at a
   pinned tag. Updating Luau is a submodule bump plus a CI verification pass.

## Non-Goals

- **Runtime execution.** This crate does not run Luau code. Use
  [mlua](https://github.com/mlua-rs/mlua) with the `luau` feature for that.
  The two crates are complementary: `luau-analyze` for pre-execution type
  checking, `mlua` for sandboxed execution.

- **Language server or IDE integration.** No incremental checking, no
  autocomplete, no hover types. These require much more of the Analysis API
  surface and are out of scope.

- **Replacing luau-analyze CLI.** The CLI tool remains useful for developer
  workflows. This crate targets programmatic embedding.

## Prior Art and Reference Projects

### luau-src-rs (mlua-rs/luau-src-rs)

https://github.com/mlua-rs/luau-src-rs

This is the actively maintained crate that vendors Luau sources and compiles them
for consumption by `mlua-sys`. Key observations:

- **Vendoring pattern.** It contains a `luau/` directory with the full Luau
  source tree. This is the pattern we follow — git submodule pointing at
  `luau-lang/luau` at a pinned tag.

- **Build approach.** It uses the `cc` crate to compile Luau's C and C++ sources
  in `build.rs`. It currently builds Luau.VM, Luau.Compiler, and Luau.Ast — but
  **not** Luau.Analysis, Luau.Config, or Luau.EqSat. We need all six.

- **Compiler flags and platform handling.** The `luau-src-rs` build script
  contains tested, working C++17 compilation flags for macOS and Linux.
  **Our `build.rs` should reference and mirror the patterns in `luau-src-rs`**
  rather than inventing them from scratch. Specifically: C++17 standard flag
  handling, warning suppression, and any Luau-specific defines like
  `LUAI_MAXCSTACK`.

- **Not a dependency.** We do not depend on `luau-src-rs` as a crate. It builds
  Luau.VM and Luau.Compiler which we don't need, and it doesn't build
  Luau.Analysis which we do. However, it is the reference implementation for
  "how to compile Luau from Rust" and should be consulted for any build system
  questions.

### Luau Web.cpp (luau-lang/luau CLI/src/Web.cpp)

The Luau project's own web playground demo. This is the closest existing example
of in-memory type checking without disk I/O. It demonstrates:

- Implementing `FileResolver` to serve source from an in-memory map
- Creating a `Frontend` with custom resolvers
- Loading builtin definitions
- Calling `frontend.check()` and collecting `CheckResult` diagnostics

Our C++ shim is essentially a production-hardened, C-API-wrapped version of this
demo.

### Luau Analysis Discussion #687

https://github.com/luau-lang/luau/discussions/687

Documents how to define types for native modules using the
`return {} :: { ... }` pattern as an alternative to definition files. Relevant
for expressing complex API types that may be awkward in `declare` syntax.

## Architecture

### Overview

```
┌──────────────────────────────────────────────────────┐
│  Your Rust Application                               │
│                                                      │
│  ┌────────────┐         ┌──────────────────────────┐ │
│  │   mlua     │         │   luau-analyze           │ │
│  │  (runtime) │         │   (type checking)        │ │
│  │            │         │                          │ │
│  │  Luau VM   │         │  Rust API (lib.rs)       │ │
│  │  Compiler  │         │    │                     │ │
│  │  Sandbox   │         │  FFI bindings (ffi.rs)   │ │
│  │            │         │    │                     │ │
│  └────────────┘         │  C shim (analyze_shim.cpp)│ │
│                         │    │                     │ │
│                         │  Luau.Analysis (C++17)   │ │
│                         │  Luau.Ast                │ │
│                         │  Luau.Config             │ │
│                         │  Luau.EqSat             │ │
│                         │  Luau.Common             │ │
│                         └──────────────────────────┘ │
└──────────────────────────────────────────────────────┘
```

The two crates are independent. They vendor Luau sources separately and link
separate static libraries. This avoids symbol conflicts and version coupling.

### Crate Structure

```
luau-analyze/
├── Cargo.toml
├── build.rs                     # Compiles Luau C++ and the shim
├── luau/                        # Git submodule -> luau-lang/luau @ pinned tag
├── shim/
│   └── analyze_shim.cpp         # C++ glue exposing extern "C" API (~250 lines)
├── src/
│   ├── lib.rs                   # Safe public Rust API (~200 lines)
│   └── ffi.rs                   # Raw extern "C" bindings
└── tests/
    └── integration.rs           # Basic type checking tests
```

### Cargo.toml

```toml
[package]
name = "luau-analyze"
version = "0.1.0"
edition = "2021"
description = "In-process Luau type checker for Rust"
license = "MIT"
links = "luau_analyze"           # Prevents duplicate linking

[build-dependencies]
cc = "1.0"
```

No runtime dependencies beyond std. The entire Luau Analysis library is compiled
and statically linked at build time.

## Build System (build.rs)

The build script compiles five Luau C++ libraries plus our shim. The source file
lists come from `luau-lang/luau/Sources.cmake` — the canonical source of truth
for which files belong to which library.

**Important:** Consult `luau-src-rs`'s `build.rs` for compiler flags before
writing ours. Key things to mirror:

- C++17 flag handling (`.std("c++17")` on cc crate)
- Warning suppression flags for Luau's C++ code
- Any `#define` macros (`LUAI_MAXCSTACK`, etc.)
- C++ stdlib linking (`-lc++` on macOS, `-lstdc++` on Linux)

```rust
use std::path::{Path, PathBuf};

fn main() {
    let luau = Path::new("luau");

    // Common compiler settings — mirror luau-src-rs patterns
    let mut base = cc::Build::new();
    base.cpp(true)
        .std("c++17")
        .warnings(false)
        .define("LUAI_MAXCSTACK", "100000");

    // TODO: Check luau-src-rs for additional compiler flags and defines.

    // ── Luau.Common ────────────────────────────────────────────
    // Tiny library, but needed by everything else.
    // Check Sources.cmake for actual file list.
    let common_includes = &[luau.join("Common/include")];

    // ── Luau.Ast ───────────────────────────────────────────────
    let ast_includes = &[
        luau.join("Ast/include"),
        luau.join("Common/include"),
    ];
    build_lib("luau_ast",
        &glob_cpp(luau.join("Ast/src")),
        ast_includes, &base);

    // ── Luau.Config ────────────────────────────────────────────
    let config_includes = &[
        luau.join("Config/include"),
        luau.join("Ast/include"),
        luau.join("Common/include"),
    ];
    build_lib("luau_config",
        &glob_cpp(luau.join("Config/src")),
        config_includes, &base);

    // ── Luau.EqSat ────────────────────────────────────────────
    let eqsat_includes = &[
        luau.join("EqSat/include"),
        luau.join("Common/include"),
    ];
    build_lib("luau_eqsat",
        &glob_cpp(luau.join("EqSat/src")),
        eqsat_includes, &base);

    // ── Luau.Analysis ──────────────────────────────────────────
    let analysis_includes = &[
        luau.join("Analysis/include"),
        luau.join("Ast/include"),
        luau.join("Config/include"),
        luau.join("EqSat/include"),
        luau.join("Common/include"),
    ];
    build_lib("luau_analysis",
        &glob_cpp(luau.join("Analysis/src")),
        analysis_includes, &base);

    // ── Our C++ shim ───────────────────────────────────────────
    build_lib("luau_analyze_shim",
        &[PathBuf::from("shim/analyze_shim.cpp")],
        analysis_includes, &base);

    // Link order matters for static libs with interdependencies
    println!("cargo:rustc-link-lib=static=luau_analyze_shim");
    println!("cargo:rustc-link-lib=static=luau_analysis");
    println!("cargo:rustc-link-lib=static=luau_eqsat");
    println!("cargo:rustc-link-lib=static=luau_config");
    println!("cargo:rustc-link-lib=static=luau_ast");

    // C++ standard library
    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=c++");
    } else if cfg!(target_os = "linux") {
        println!("cargo:rustc-link-lib=stdc++");
    }

    // Rebuild if shim changes
    println!("cargo:rerun-if-changed=shim/analyze_shim.cpp");
}

fn build_lib(name: &str, sources: &[PathBuf], includes: &[PathBuf],
             base: &cc::Build) {
    let mut build = base.clone();
    for inc in includes {
        build.include(inc);
    }
    for src in sources {
        build.file(src);
    }
    build.compile(name);
}

fn glob_cpp(dir: PathBuf) -> Vec<PathBuf> {
    std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |ext| ext == "cpp"))
        .collect()
}
```

**Open question:** Luau.Common may have `.cpp` files that Analysis depends on
but that aren't in Ast. Verify against `Sources.cmake` and add a `build_lib`
call for Common if needed.

## C++ Shim (shim/analyze_shim.cpp)

This is the core bridge — approximately 250 lines of C++ that hides the
template-heavy Analysis internals behind a flat `extern "C"` API. The design is
modeled on Luau's own `Web.cpp` demo, which does in-memory type checking for the
luau.org playground.

```cpp
#include "Luau/Frontend.h"
#include "Luau/BuiltinDefinitions.h"
#include "Luau/Config.h"
#include "Luau/ModuleResolver.h"

#include <string>
#include <map>
#include <optional>
#include <vector>
#include <cstring>

// ── C ABI types ────────────────────────────────────────────────

extern "C" {

struct LuauDiagnostic {
    uint32_t line;         // 0-based
    uint32_t col;          // 0-based
    uint32_t end_line;
    uint32_t end_col;
    uint32_t severity;     // 0 = error, 1 = warning
    const char* message;   // Owned by CheckResultStorage
    uint32_t message_len;
};

struct LuauCheckResult {
    void* _internal;                // Opaque pointer to CheckResultStorage
    const LuauDiagnostic* diagnostics;
    uint32_t diagnostic_count;
};

typedef struct LuauChecker LuauChecker;

LuauChecker* luau_checker_new(void);
void luau_checker_free(LuauChecker* checker);

int luau_checker_add_definitions(
    LuauChecker* checker,
    const char* defs,
    uint32_t defs_len
);

LuauCheckResult luau_checker_check(
    LuauChecker* checker,
    const char* source,
    uint32_t source_len
);

void luau_check_result_free(LuauCheckResult result);

} // extern "C"


// ── Internal: in-memory file resolver ──────────────────────────

// Modeled on DemoFileResolver from luau-lang/luau CLI/src/Web.cpp.
// Serves source from an in-memory map — no disk I/O.
struct InMemoryFileResolver : Luau::FileResolver
{
    std::map<std::string, std::string> sources;

    std::optional<Luau::SourceCode> readSource(
        const Luau::ModuleName& name
    ) override {
        auto it = sources.find(name);
        if (it == sources.end())
            return std::nullopt;
        return Luau::SourceCode{it->second, Luau::SourceCode::Module};
    }

    std::optional<Luau::ModuleInfo> resolveModule(
        const Luau::ModuleInfo* context,
        Luau::AstExpr* expr,
        const Luau::TypeCheckLimits& limits
    ) override {
        if (auto* g = expr->as<Luau::AstExprGlobal>())
            return Luau::ModuleInfo{g->name.value};
        if (auto* s = expr->as<Luau::AstExprConstantString>())
            return Luau::ModuleInfo{
                std::string(s->value.data, s->value.size)
            };
        return std::nullopt;
    }

    std::string getHumanReadableModuleName(
        const Luau::ModuleName& name
    ) const override {
        return name;
    }
};


// ── Internal: strict config resolver ───────────────────────────

struct StrictConfigResolver : Luau::ConfigResolver
{
    Luau::Config config;

    StrictConfigResolver() {
        config.mode = Luau::Mode::Strict;
    }

    const Luau::Config& getConfig(
        const Luau::ModuleName&,
        const Luau::TypeCheckLimits&
    ) const override {
        return config;
    }
};


// ── Internal: checker and result state ─────────────────────────

struct LuauChecker {
    InMemoryFileResolver fileResolver;
    StrictConfigResolver configResolver;
    Luau::FrontendOptions options;
    std::unique_ptr<Luau::Frontend> frontend;

    LuauChecker() {
        options.retainFullTypeGraphs = false;
        frontend = std::make_unique<Luau::Frontend>(
            &fileResolver, &configResolver, options
        );
        // Register Luau builtins (print, type, string.*, math.*, etc.)
        Luau::unfreeze(frontend->globals.globalTypes);
        Luau::registerBuiltinGlobals(*frontend, frontend->globals);
        Luau::freeze(frontend->globals.globalTypes);
    }
};

struct CheckResultStorage {
    std::vector<std::string> messages;
    std::vector<LuauDiagnostic> diagnostics;
};


// ── C API implementation ───────────────────────────────────────

extern "C" {

LuauChecker* luau_checker_new(void) {
    return new LuauChecker();
}

void luau_checker_free(LuauChecker* checker) {
    delete checker;
}

int luau_checker_add_definitions(
    LuauChecker* checker,
    const char* defs,
    uint32_t defs_len
) {
    std::string source(defs, defs_len);
    Luau::unfreeze(checker->frontend->globals.globalTypes);
    auto result = checker->frontend->loadDefinitionFile(
        checker->frontend->globals,
        checker->frontend->globals.globalScope,
        source,
        "@definitions",
        false,    // captureComments
        false     // typeCheckForAutocomplete
    );
    Luau::freeze(checker->frontend->globals.globalTypes);
    return result.success ? 0 : 1;
}

LuauCheckResult luau_checker_check(
    LuauChecker* checker,
    const char* source,
    uint32_t source_len
) {
    // Reset module state; global definitions persist
    checker->frontend->clear();
    checker->fileResolver.sources.clear();
    checker->fileResolver.sources["main"] =
        std::string(source, source_len);

    // Type check
    Luau::CheckResult cr = checker->frontend->check("main");

    // Lint
    Luau::LintResult lr = checker->frontend->lint("main");

    // Collect diagnostics
    auto* storage = new CheckResultStorage();

    for (auto& error : cr.errors) {
        storage->messages.push_back(Luau::toString(error));
        storage->diagnostics.push_back(LuauDiagnostic{
            .line     = error.location.begin.line,
            .col      = error.location.begin.column,
            .end_line = error.location.end.line,
            .end_col  = error.location.end.column,
            .severity = 0,
            .message  = nullptr,
            .message_len = 0,
        });
    }

    for (auto& warning : lr.warnings) {
        storage->messages.push_back(warning.text);
        storage->diagnostics.push_back(LuauDiagnostic{
            .line     = warning.location.begin.line,
            .col      = warning.location.begin.column,
            .end_line = warning.location.end.line,
            .end_col  = warning.location.end.column,
            .severity = 1,
            .message  = nullptr,
            .message_len = 0,
        });
    }

    // Patch message pointers now that vectors are stable
    for (size_t i = 0; i < storage->diagnostics.size(); i++) {
        storage->diagnostics[i].message =
            storage->messages[i].c_str();
        storage->diagnostics[i].message_len =
            (uint32_t)storage->messages[i].size();
    }

    return LuauCheckResult{
        ._internal        = storage,
        .diagnostics      = storage->diagnostics.data(),
        .diagnostic_count = (uint32_t)storage->diagnostics.size(),
    };
}

void luau_check_result_free(LuauCheckResult result) {
    delete static_cast<CheckResultStorage*>(result._internal);
}

} // extern "C"
```

### Shim API notes

- `luau_checker_new` creates a Frontend with builtins registered. The Frontend
  is reused across checks — only module state is cleared between calls.

- `luau_checker_add_definitions` loads type definitions into the global scope.
  These persist across checks. Call it once at startup with your API definitions.

- `luau_checker_check` clears module state, sets source for "main", runs
  `frontend->check("main")` and `frontend->lint("main")`, and collects all
  diagnostics. The returned `LuauCheckResult` owns the diagnostic data and must
  be freed with `luau_check_result_free`.

- The `loadDefinitionFile` API signature may change across Luau versions. The
  arguments shown here match the current `Web.cpp` demo patterns. Verify against
  the actual `Frontend.h` header in the pinned Luau version.

## Rust API

### src/ffi.rs — raw bindings (not public)

```rust
use std::ffi::c_void;

#[repr(C)]
pub(crate) struct LuauDiagnostic {
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub severity: u32,
    pub message: *const u8,
    pub message_len: u32,
}

#[repr(C)]
pub(crate) struct LuauCheckResult {
    pub _internal: *mut c_void,
    pub diagnostics: *const LuauDiagnostic,
    pub diagnostic_count: u32,
}

pub(crate) enum LuauChecker {}

extern "C" {
    pub fn luau_checker_new() -> *mut LuauChecker;
    pub fn luau_checker_free(checker: *mut LuauChecker);
    pub fn luau_checker_add_definitions(
        checker: *mut LuauChecker,
        defs: *const u8,
        defs_len: u32,
    ) -> i32;
    pub fn luau_checker_check(
        checker: *mut LuauChecker,
        source: *const u8,
        source_len: u32,
    ) -> LuauCheckResult;
    pub fn luau_check_result_free(result: LuauCheckResult);
}
```

### src/lib.rs — safe public API

```rust
mod ffi;

/// Severity of a type checking diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// A single diagnostic from the Luau type checker or linter.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub severity: Severity,
    pub message: String,
}

/// Result of type-checking a Luau script.
#[derive(Debug, Clone)]
pub struct CheckResult {
    pub diagnostics: Vec<Diagnostic>,
}

impl CheckResult {
    /// Returns true if there are no errors (warnings are allowed).
    pub fn is_ok(&self) -> bool {
        !self.diagnostics.iter().any(|d| d.severity == Severity::Error)
    }

    /// Returns only error diagnostics.
    pub fn errors(&self) -> Vec<&Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect()
    }

    /// Returns only warning diagnostics.
    pub fn warnings(&self) -> Vec<&Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .collect()
    }
}

/// A reusable Luau type checker instance.
///
/// Create one, load your API definitions once, then call `check()`
/// repeatedly for each script.
pub struct Checker {
    inner: *mut ffi::LuauChecker,
}

unsafe impl Send for Checker {}
// Not Sync — the C++ Frontend is single-threaded.

impl Checker {
    /// Create a new type checker with Luau builtins registered.
    pub fn new() -> Self {
        let inner = unsafe { ffi::luau_checker_new() };
        assert!(!inner.is_null(), "failed to create Luau checker");
        Self { inner }
    }

    /// Load global type definitions describing your host API.
    ///
    /// Uses Luau definition file syntax:
    /// ```text
    /// declare class Todo
    ///     content: string?
    ///     done: boolean
    ///     function complete(self): ()
    /// end
    /// ```
    ///
    /// Can be called multiple times to accumulate definitions.
    /// Definitions persist across `check()` calls.
    pub fn add_definitions(&mut self, defs: &str) -> Result<(), String> {
        let result = unsafe {
            ffi::luau_checker_add_definitions(
                self.inner,
                defs.as_ptr(),
                defs.len() as u32,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err("failed to load Luau type definitions".into())
        }
    }

    /// Type-check a Luau script in --!strict mode.
    ///
    /// The source should include `--!strict` at the top for full
    /// strictness, though the checker's config resolver enforces
    /// strict mode regardless.
    pub fn check(&mut self, source: &str) -> CheckResult {
        let raw = unsafe {
            ffi::luau_checker_check(
                self.inner,
                source.as_ptr(),
                source.len() as u32,
            )
        };

        let diagnostics = if raw.diagnostic_count == 0 {
            Vec::new()
        } else {
            let slice = unsafe {
                std::slice::from_raw_parts(
                    raw.diagnostics,
                    raw.diagnostic_count as usize,
                )
            };
            slice.iter().map(|d| {
                let message = unsafe {
                    let bytes = std::slice::from_raw_parts(
                        d.message, d.message_len as usize,
                    );
                    String::from_utf8_lossy(bytes).into_owned()
                };
                Diagnostic {
                    line: d.line,
                    col: d.col,
                    end_line: d.end_line,
                    end_col: d.end_col,
                    severity: if d.severity == 0 {
                        Severity::Error
                    } else {
                        Severity::Warning
                    },
                    message,
                }
            }).collect()
        };

        unsafe { ffi::luau_check_result_free(raw) };

        CheckResult { diagnostics }
    }
}

impl Drop for Checker {
    fn drop(&mut self) {
        unsafe { ffi::luau_checker_free(self.inner) };
    }
}

impl Default for Checker {
    fn default() -> Self {
        Self::new()
    }
}
```

## Usage Example

```rust
use luau_analyze::{Checker, Severity};

fn main() {
    let mut checker = Checker::new();

    // Load API type definitions once
    checker.add_definitions(r#"
        declare class Todo
            content: string?
            done: boolean
            due: string?
            function complete(self): ()
            function cancel(self): ()
        end

        declare class TodoBuilder
            function under(self, parent: any): TodoBuilder
            function content(self, content: string): TodoBuilder
            function due(self, due: string): TodoBuilder
            function save(self): Todo
        end

        declare Todo: {
            create: () -> TodoBuilder,
        }
    "#).expect("definitions should parse");

    // Type-check a script — this catches due(42) as a type error
    let result = checker.check(r#"
        --!strict
        local todo = Todo.create()
            :content("Review notes")
            :due(42)
            :save()
    "#);

    for diag in &result.diagnostics {
        eprintln!(
            "{}:{}: [{:?}] {}",
            diag.line + 1, diag.col + 1, diag.severity, diag.message
        );
    }

    assert!(!result.is_ok(), "expected type error for due(42)");
}
```

## Risks and Open Questions

### Build system complexity

The `cc` crate handles C++17 on clang and gcc, but Luau's Analysis code is
substantially larger than VM+Compiler. Compilation may surface issues not present
in `luau-src-rs`'s existing build.

**Mitigation:** Mirror `luau-src-rs`'s compiler flag patterns exactly. Start
with macOS (clang), then verify Linux (gcc/clang). Start with the simplest
possible build and add flags only when compilation fails.

### Luau Analysis API stability

The Analysis library is an internal API with no stability guarantees. The core
`Frontend` interface (`check`, `lint`, `loadDefinitionFile`, `FileResolver`) has
been stable since Luau's open-source release, but could change.

**Mitigation:** Pin to specific Luau tags. Test against the pinned version in
CI. The shim's API surface is narrow enough that adapting to upstream changes
should be straightforward.

### Definition file expressiveness

Luau's `declare` syntax is less documented than the type annotation syntax. It
may be challenging to express complex patterns like:

- Builder chains that return `Self`
- Union types (like a `Node` enum with downcasting)
- Trait-like polymorphism (like `NodeApi` methods available on all node types)
- Generic type parameters

**Mitigation:** Two approaches exist. The `declare class` / `declare function`
syntax is the standard approach. If that proves limiting, the alternative
`return {} :: { ... }` pattern (documented in luau-lang/luau Discussion #687)
uses regular Luau type syntax within a virtual module and may handle complex
cases better. Spike the definition file format early — this is the highest-risk
unknown.

### Checker reuse semantics

The shim calls `frontend->clear()` before each check, resetting module state
while preserving global definitions. This needs verification: do definitions
loaded via `loadDefinitionFile` actually survive `clear()`? The Web.cpp demo
suggests yes (it calls `frontend.clear()` and `fileResolver.source.clear()`
between checks while builtins persist), but this should be tested explicitly.

### loadDefinitionFile API drift

The `loadDefinitionFile` function signature has changed across Luau versions.
The shim uses arguments matching the current codebase, but older or newer
versions may differ. The parameters to verify against the pinned version:
- Global scope reference
- Source string
- Module name
- Boolean flags for comments and autocomplete

### What this crate does NOT provide

- **No runtime execution.** Use mlua for that.
- **No LSP / autocomplete.** Just batch checking.
- **No incremental checking.** Each `check()` re-checks from scratch.
  Fine for scripts under ~1000 lines.
- **Not `Sync`.** Create separate `Checker` instances for concurrent checks.
- **No new/old solver choice.** Uses whichever solver is the Luau default for
  the pinned version. (The new solver is becoming the default.)

## Development Plan

### Phase 1: Build spike

Get the Luau Analysis library compiling from `build.rs` on macOS. No shim, no
Rust API — just verify the `cc` crate can build all ~40 Analysis source files
with the right includes and flags.

### Phase 2: Minimal shim

Write the shim with just `luau_checker_new` and `luau_checker_check`. Verify
that checking `local x: number = "hello"` in strict mode returns a type error.
This proves the full pipeline works: Rust → C FFI → C++ Frontend → diagnostics
back to Rust.

### Phase 3: Definition loading

Add `luau_checker_add_definitions`. Test with a small type definition and verify
that scripts using the defined types are checked correctly. This is where
definition file expressiveness gets tested.

### Phase 4: API definition spike

Translate a representative slice of the target application's Rust API into Luau
type definitions. Test with scripts that exercise builder patterns, union types,
and method chains. This validates that Luau's type system can faithfully
represent the API.

### Phase 5: Linux support

Verify build and tests pass on Linux (gcc and clang). Set up CI for both macOS
and Linux.

### Phase 6: Polish and publish

Error messages, documentation, crate publishing, integration with the target
application.
