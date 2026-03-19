# Public Release Cleanup Plan

This plan turns the public-release review into an execution checklist. The
current codebase is mechanically healthy (`cargo test`, `clippy`, `cargo doc`,
`xtask smoke`, and `cargo publish --dry-run` all pass), but the repository
still reads like an internal/pre-alpha project and the packaged crate ships far
more Luau submodule material than the build actually needs.

The goal is to make the repository and published artifacts look intentional to
external users: clear release posture, correct licensing, minimal package
contents, explicit support boundaries, and a small public API with no obvious
first-release footguns.

1. Stage One: Lock Public Release Posture

Resolve the mismatch between "public release" and the current internal/pre-alpha
messaging before changing code or packaging policy.

1. [ ] Rewrite [README.md](/Users/cortesi/git/public/luau-analyze/README.md) so
       the status, audience, and support expectations match the intended public
       release.
2. [ ] Update
       [docs/release-checklist.md](/Users/cortesi/git/public/luau-analyze/docs/release-checklist.md)
       from an internal-consumer checklist to a public release checklist.
3. [ ] Review the checked-in planning docs under
       [plans/](/Users/cortesi/git/public/luau-analyze/plans) and decide which
       should remain public, which should be rewritten, and which should be
       removed from the repository entirely. This is a repository-visibility
       decision for GitHub users, not a crates.io packaging change.
4. [ ] Remove or rewrite stale internal-only statements such as the
       "internal/private consumption first" release note in
       [plans/proj.md](/Users/cortesi/git/public/luau-analyze/plans/proj.md).
5. [ ] Decide and record the intended platform support posture before any code
       cleanup: if Windows/MSVC is unsupported, enforce that in code and docs;
       if it is experimental or supported, update docs and release gates to
       match reality.
6. [ ] Record the first-release API decisions up front for the main open design
       points: `Checker::default()`, the public `CheckerOptions` shape, and
       whether `check_with_options` should surface boundary failures as
       diagnostics or structured Rust errors.

2. Stage Two: Fix Licensing And Third-Party Notice Story

Make the repository legally and operationally clear to outside users.

1. [ ] Add a root
       [LICENSE](/Users/cortesi/git/public/luau-analyze/LICENSE) file matching
       the workspace manifest license declaration in
       [Cargo.toml](/Users/cortesi/git/public/luau-analyze/Cargo.toml).
2. [ ] Audit the pinned Luau submodule under
       [crates/luau-analyze/luau](/Users/cortesi/git/public/luau-analyze/crates/luau-analyze/luau)
       and document which upstream licenses must remain visible in the repo and
       in published artifacts.
3. [ ] Add a short third-party licensing section to
       [README.md](/Users/cortesi/git/public/luau-analyze/README.md) or a
       dedicated notice file so users understand that Luau is pinned as a git
       submodule in the repo and bundled in the published crate.

3. Stage Three: Minimize Published Crate Contents

Reduce the `luau-analyze` crates.io artifact to the subset needed to build and
use the library.

1. [ ] Audit the current publish set with `cargo package --list -p
       luau-analyze` and map it against the actual inputs referenced by
       [crates/luau-analyze/build.rs](/Users/cortesi/git/public/luau-analyze/crates/luau-analyze/build.rs).
2. [ ] Add an explicit `include` or `exclude` policy in
       [crates/luau-analyze/Cargo.toml](/Users/cortesi/git/public/luau-analyze/crates/luau-analyze/Cargo.toml)
       so the package is derived from actual `build.rs` inputs plus required
       metadata/license files, instead of maintaining a hand-written denylist
       of Luau directories.
3. [ ] Re-run `cargo package --list -p luau-analyze` and
       `cargo publish --dry-run -p luau-analyze` to confirm the package still
       builds, required upstream license files remain present, and the artifact
       size drops materially.
4. [ ] Document the vendoring/package policy in
       [docs/luau-update-playbook.md](/Users/cortesi/git/public/luau-analyze/docs/luau-update-playbook.md)
       so future Luau submodule bumps do not silently re-expand the publish set.

4. Stage Four: Tighten Package Metadata And Publish Policy

Make the workspace manifests look complete and deliberate to crates.io users.

1. [ ] Add missing package metadata for
       [crates/luau-analyze/Cargo.toml](/Users/cortesi/git/public/luau-analyze/crates/luau-analyze/Cargo.toml):
       `readme`, `documentation`, `keywords`, `categories`, and `rust-version`.
2. [ ] Decide whether `lan` is intended to be published; if not, mark
       [crates/lan/Cargo.toml](/Users/cortesi/git/public/luau-analyze/crates/lan/Cargo.toml)
       with `publish = false`.
3. [ ] Mark
       [xtask/Cargo.toml](/Users/cortesi/git/public/luau-analyze/xtask/Cargo.toml)
       with `publish = false` unless there is a real external publishing story
       for it.
4. [ ] Review the root workspace metadata in
       [Cargo.toml](/Users/cortesi/git/public/luau-analyze/Cargo.toml) and add
       any shared package fields that should be inherited consistently.
5. [ ] Add a first public release note or
       [CHANGELOG.md](/Users/cortesi/git/public/luau-analyze/CHANGELOG.md) so
       external users can see the initial version policy and what `0.1.0`
       contains.

5. Stage Five: Improve External User Documentation

Add the missing operational context that a public user needs before trying the
crate.

1. [ ] Extend [README.md](/Users/cortesi/git/public/luau-analyze/README.md)
       with a real "use this crate" section that shows a dependency snippet and
       a minimal Rust example for library consumers.
2. [ ] Document build prerequisites for public users: supported platforms,
       required C/C++ toolchain, and the submodule requirement for git checkouts
       implied by
       [crates/luau-analyze/build.rs](/Users/cortesi/git/public/luau-analyze/crates/luau-analyze/build.rs).
3. [ ] Keep the current support boundary explicit in
       [docs/compatibility.md](/Users/cortesi/git/public/luau-analyze/docs/compatibility.md):
       single-file checks only, no cross-file `require` support, strict-only
       mode, solver fixed to `new`, and a Windows statement that matches the
       Stage 1 platform decision and actual build behavior.
4. [ ] Add a short troubleshooting section covering the common failure modes:
       missing submodule, unsupported toolchain/platform, and packaged-vs-git
       checkout differences.

6. Stage Six: Harden First-Release Public API Edges

Make a small number of API decisions now, before outside users start depending
on them.

1. [ ] Implement the recorded Stage 1 API decisions consistently across code,
       tests, and docs so execution does not stall on ad hoc design calls.
2. [ ] Review `impl Default for Checker` in
       [crates/luau-analyze/src/lib.rs](/Users/cortesi/git/public/luau-analyze/crates/luau-analyze/src/lib.rs)
       and decide whether to remove it or keep it with a very explicit
       justification; the current implementation panics on failed native init.
3. [ ] Revisit the public configuration structs in
       [crates/luau-analyze/src/lib.rs](/Users/cortesi/git/public/luau-analyze/crates/luau-analyze/src/lib.rs),
       especially `CheckerOptions`, to confirm the exposed fields are the API
       shape you want to freeze for early external consumers.
4. [ ] Decide whether FFI-boundary failures during `check_with_options` should
       remain synthetic diagnostics or become structured Rust errors, and update
       docs/tests accordingly.
5. [ ] Add or strengthen tests and docs around the unsafe thread-safety claims
       for `Checker: Send` and `CancellationToken: Send + Sync`.

7. Stage Seven: Add Release Gates For Packaging And Docs

Prevent regressions in the newly cleaned-up public surface.

1. [ ] Extend [.github/workflows/ci.yml](/Users/cortesi/git/public/luau-analyze/.github/workflows/ci.yml)
       with a packaging gate that runs `cargo publish --dry-run -p
       luau-analyze`.
2. [ ] Add a package-contents gate that checks `cargo package --list -p
       luau-analyze` against the intended vendored file set.
3. [ ] Treat public docs as a release gate by keeping `cargo doc -p
       luau-analyze --no-deps` in CI and optionally adding a rustdoc warning
       policy if the team wants stricter enforcement.
4. [ ] Update
       [docs/release-checklist.md](/Users/cortesi/git/public/luau-analyze/docs/release-checklist.md)
       so the final release checklist matches the CI gates and manual release
       process exactly.

8. Stage Eight: Final Public Release Verification

Run the cleaned-up release flow end to end and confirm the output is something
you actually want to publish.

1. [ ] Re-run the full local validation set: `cargo test --workspace`, `cargo
       clippy --workspace --all-targets --all-features -- -D warnings`, `cargo
       run -p xtask -- smoke`, `cargo doc -p luau-analyze --no-deps`, and
       `cargo publish --dry-run -p luau-analyze`.
2. [ ] Inspect the final packaged crate tarball contents, size, and license
       files rather than relying on command success alone.
3. [ ] Smoke-test the crate from a clean downstream Rust project using the
       intended public consumption path.
4. [ ] Tag the release only after the repository docs, manifests, and published
       artifact all tell the same story.

## Approval Gate

Review and edit this plan before implementation starts. After approval, execution
should proceed stage by stage, and any commit should be approved before it is
created unless that requirement is explicitly waived.
