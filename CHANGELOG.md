# Changelog

## [0.1.0] - 2026-03-19

### Added
- First public release of `luau-analyze`.
- In-process static type checking for Luau scripts using the `Analysis` C++ frontend.
- Native safe Rust wrapper for `LuauCheckOptions` and `LuauCheckResult`.
- Support for host-provided definition files through `Checker::add_definitions`.
- Strict mode is enforced for all checker instances.
- Single-module real-time policy without queue/batch features.
- Support for execution timeout limits and threaded cancellation tokens.

### Changed
- Refactored `Checker::check` and `Checker::check_with_options` to return `Result<CheckResult, Error>`.
- Removed panicking `Checker::default()` implementation.
- Fixed FFI failure mappings to return structured errors rather than synthetic diagnostic entries.