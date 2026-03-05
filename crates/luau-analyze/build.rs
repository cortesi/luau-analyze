//! Build script for `luau-analyze`.
//!
//! Compiles Luau C++ libraries and the local C shim into static libraries.

use std::{
    env, fs,
    path::{Path, PathBuf},
    slice::from_ref,
};

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

    let common_include = luau_root.join("Common/include");
    let ast_include = luau_root.join("Ast/include");
    let compiler_include = luau_root.join("Compiler/include");
    let vm_include = luau_root.join("VM/include");
    let config_include = luau_root.join("Config/include");
    let analysis_include = luau_root.join("Analysis/include");

    // Mirror the proven luau-src-rs setup with C++17 and shared defines.
    let mut base = cc::Build::new();
    base.cpp(true)
        .std("c++17")
        .warnings(false)
        .define("LUAI_MAXCSTACK", "1000000")
        .define("LUA_VECTOR_SIZE", "3")
        .define("LUA_API", "extern \"C\"")
        .define("LUACODE_API", "extern \"C\"");

    if cfg!(debug_assertions) {
        base.define("LUAU_ENABLE_ASSERT", None);
    } else {
        base.flag_if_supported("-fno-math-errno");
    }

    let common_sources = collect_cpp_sources(&luau_root.join("Common/src"));
    let ast_sources = collect_cpp_sources(&luau_root.join("Ast/src"));
    let vm_sources = collect_cpp_sources(&luau_root.join("VM/src"));
    let compiler_sources = collect_cpp_sources(&luau_root.join("Compiler/src"));
    let config_sources = collect_cpp_sources(&luau_root.join("Config/src"));
    let analysis_sources = collect_cpp_sources(&luau_root.join("Analysis/src"));

    build_cpp_library(
        "luau_common",
        &common_sources,
        from_ref(&common_include),
        &base,
    );
    build_cpp_library(
        "luau_ast",
        &ast_sources,
        &[ast_include.clone(), common_include.clone()],
        &base,
    );
    build_cpp_library(
        "luau_vm",
        &vm_sources,
        &[vm_include.clone(), common_include.clone()],
        &base,
    );
    build_cpp_library(
        "luau_compiler",
        &compiler_sources,
        &[
            compiler_include.clone(),
            ast_include.clone(),
            common_include.clone(),
        ],
        &base,
    );
    build_cpp_library(
        "luau_config",
        &config_sources,
        &[
            config_include.clone(),
            vm_include.clone(),
            compiler_include.clone(),
            ast_include.clone(),
            common_include.clone(),
        ],
        &base,
    );
    build_cpp_library(
        "luau_analysis",
        &analysis_sources,
        &[
            analysis_include.clone(),
            config_include.clone(),
            vm_include.clone(),
            compiler_include.clone(),
            ast_include.clone(),
            common_include.clone(),
        ],
        &base,
    );
    build_cpp_library(
        "luau_analyze_shim",
        &[PathBuf::from("shim/analyze_shim.cpp")],
        &[
            analysis_include,
            config_include,
            vm_include,
            compiler_include,
            ast_include,
            common_include,
        ],
        &base,
    );

    // Static-link order is significant.
    println!("cargo:rustc-link-lib=static=luau_analyze_shim");
    println!("cargo:rustc-link-lib=static=luau_analysis");
    println!("cargo:rustc-link-lib=static=luau_config");
    println!("cargo:rustc-link-lib=static=luau_compiler");
    println!("cargo:rustc-link-lib=static=luau_vm");
    println!("cargo:rustc-link-lib=static=luau_ast");
    println!("cargo:rustc-link-lib=static=luau_common");

    let target = env::var("TARGET").unwrap_or_default();
    let host = env::var("HOST").unwrap_or_default();
    if let Some(stdlib) = cpp_stdlib(&target, &host) {
        println!("cargo:rustc-link-lib={stdlib}");
    }
}

/// Compiles a single static C++ library from source files and include roots.
fn build_cpp_library(name: &str, sources: &[PathBuf], includes: &[PathBuf], base: &cc::Build) {
    if sources.is_empty() {
        panic!("no sources found for `{name}`");
    }

    let mut build = base.clone();
    for include in includes {
        build.include(include);
    }
    for source in sources {
        build.file(source);
    }
    build.compile(name);
}

/// Collects and deterministically sorts all `.cpp` files in a source directory.
fn collect_cpp_sources(dir: &Path) -> Vec<PathBuf> {
    let mut sources: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("failed to read `{}`: {error}", dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "cpp"))
        .collect();
    sources.sort();
    sources
}

/// Determines the C++ standard library to link for the current target.
fn cpp_stdlib(target: &str, host: &str) -> Option<String> {
    let kind = if host == target { "HOST" } else { "TARGET" };
    let env_value = env::var(format!("CXXSTDLIB_{target}"))
        .or_else(|_| env::var(format!("CXXSTDLIB_{}", target.replace('-', "_"))))
        .or_else(|_| env::var(format!("{kind}_CXXSTDLIB")))
        .or_else(|_| env::var("CXXSTDLIB"))
        .ok();

    if env_value.is_some() {
        return env_value;
    }

    if target.contains("msvc") {
        None
    } else if target.contains("apple") || target.contains("freebsd") || target.contains("openbsd") {
        Some("c++".to_owned())
    } else if target.contains("android") {
        Some("c++_shared".to_owned())
    } else {
        Some("stdc++".to_owned())
    }
}
