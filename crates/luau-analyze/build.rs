//! Build script for `luau-analyze`.
//!
//! Compiles the vendored Luau sources and local shim into a private shared library
//! that the Rust crate loads at runtime. Keeping the analyzer in its own dynamic
//! library isolates Luau symbols from other embedders such as `mlua`.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Vendored Luau components compiled into the native checker.
const LUAU_COMPONENTS: [&str; 6] = ["Common", "Ast", "VM", "Compiler", "Config", "Analysis"];

/// Entry point for the build script.
fn main() {
    let luau_root = Path::new("luau");
    if !luau_root.join("Sources.cmake").exists() {
        panic!(
            "missing Luau sources at `{}`; run `git submodule update --init --recursive`",
            luau_root.display()
        );
    }

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=luau");
    println!("cargo:rerun-if-changed=shim/analyze_shim.cpp");

    let target = env::var("TARGET").unwrap_or_default();

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .warnings(false)
        .define("LUAI_MAXCSTACK", "1000000")
        .define("LUA_VECTOR_SIZE", "3")
        .define("LUA_API", "extern \"C\"")
        .define("LUACODE_API", "extern \"C\"");
    for component in LUAU_COMPONENTS {
        build.include(luau_root.join(component).join("include"));
    }

    if cfg!(debug_assertions) {
        build.define("LUAU_ENABLE_ASSERT", None);
    } else {
        build.flag_if_supported("-fno-math-errno");
    }

    if target.contains("windows-msvc") {
        build.define("LUAU_ANALYZE_EXPORT", "__declspec(dllexport)");
    } else {
        build
            .flag_if_supported("-fPIC")
            .flag_if_supported("-fvisibility=hidden")
            .flag_if_supported("-fvisibility-inlines-hidden")
            .define(
                "LUAU_ANALYZE_EXPORT",
                "__attribute__((visibility(\"default\")))",
            );
    }

    let mut sources = Vec::new();
    for component in LUAU_COMPONENTS {
        sources.extend(collect_cpp_sources(&luau_root.join(component).join("src")));
    }
    sources.push(PathBuf::from("shim/analyze_shim.cpp"));

    let compiler = build.get_compiler();
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR should be set"));
    let native_library = native_library_path(&out_dir, &target);
    build_native_library(&compiler, &sources, &native_library, &target);
    println!(
        "cargo:rustc-env=LUAU_ANALYZE_NATIVE_LIB_PATH={}",
        native_library.display()
    );
    println!(
        "cargo:rustc-env=LUAU_ANALYZE_NATIVE_LIB_FILE_NAME={}",
        native_library
            .file_name()
            .and_then(|file_name| file_name.to_str())
            .expect("native library path should end with a UTF-8 file name")
    );
}

/// Compiles the native checker shared library.
fn build_native_library(compiler: &cc::Tool, sources: &[PathBuf], output: &Path, target: &str) {
    let mut command = Command::new(compiler.path());
    command.args(compiler.args());

    if target.contains("windows-msvc") {
        command.arg("/LD");
        command.arg(format!("/Fe:{}", output.display()));
    } else if target.contains("apple") {
        command.arg("-dynamiclib");
        command.arg("-o");
        command.arg(output);
    } else {
        command.arg("-shared");
        command.arg("-o");
        command.arg(output);
    }

    for source in sources {
        command.arg(source);
    }

    let result = command
        .output()
        .unwrap_or_else(|error| panic!("failed to spawn `{}`: {error}", compiler.path().display()));

    if !result.status.success() {
        let stdout = String::from_utf8_lossy(&result.stdout);
        let stderr = String::from_utf8_lossy(&result.stderr);
        panic!(
            "failed to build native checker library `{}`\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.display()
        );
    }
}

/// Collects and deterministically sorts all `.cpp` files in a source directory.
fn collect_cpp_sources(dir: &Path) -> Vec<PathBuf> {
    let mut sources: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("failed to read `{}`: {error}", dir.display()))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| {
                    panic!("failed to read entry in `{}`: {error}", dir.display())
                })
                .path()
        })
        .filter(|path| path.extension().is_some_and(|ext| ext == "cpp"))
        .collect();
    sources.sort();
    sources
}

/// Returns the platform-specific path for the private native checker library.
fn native_library_path(out_dir: &Path, target: &str) -> PathBuf {
    let (prefix, suffix) = if target.contains("windows") {
        ("", "dll")
    } else if target.contains("apple") {
        ("lib", "dylib")
    } else {
        ("lib", "so")
    };

    out_dir.join(format!("{prefix}luau_analyze_native.{suffix}"))
}
