# Compatibility and Versioning

## Platform Support

Current target platforms:

- macOS (clang)
- Linux (gcc/clang)

Windows is currently out of scope.

## Luau Compatibility

- Luau is vendored as a submodule and pinned per release line.
- `v0.1` targets Luau tag `0.710`.
- Any Luau bump requires rerunning the update playbook and full test matrix.

## API Stability Policy

- Pre-1.0 releases may adjust API shape to improve safety and ergonomics.
- Public API changes are documented in release notes.
- We keep the external API intentionally small:
  - `Checker`
  - `CheckResult`
  - `Diagnostic`
  - `Severity`
  - `Error`

## Crate Versioning Policy

- Semantic Versioning is used for crate releases.
- Before `1.0.0`, breaking changes may occur in minor releases.
- Patch releases should remain source-compatible and focus on fixes.
