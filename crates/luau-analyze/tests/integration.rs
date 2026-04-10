//! Integration tests for `luau-analyze`.

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::Duration,
    };

    use luau_analyze::{CancellationToken, CheckOptions, Checker, Severity, VirtualModule};
    use mlua::Lua;

    /// Expected result marker parsed from script header comments.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Expectation {
        /// Script should type-check without errors.
        Pass,
        /// Script should report at least one error.
        Fail,
    }

    /// Verifies strict-mode type mismatch reporting.
    #[test]
    fn strict_type_mismatch_reports_error() {
        let mut checker = Checker::new().expect("checker creation should succeed");
        let result = checker
            .check(
                r#"
            --!strict
            local x: number = "hello"
            "#,
            )
            .unwrap();

        assert!(!result.is_ok(), "expected strict type mismatch");
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == Severity::Error)
        );
        assert!(!result.timed_out);
        assert!(!result.cancelled);
    }

    /// Verifies strict mode is enforced even without `--!strict`.
    #[test]
    fn strict_mode_is_enforced_without_hot_comment() {
        let mut checker = Checker::new().expect("checker creation should succeed");
        let result = checker
            .check(
                r#"
            local x: number = "hello"
            "#,
            )
            .unwrap();

        assert!(!result.is_ok(), "strict type mismatch should be reported");
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == Severity::Error)
        );
    }

    /// Verifies invalid definitions return an actionable error.
    #[test]
    fn invalid_definitions_fail() {
        let mut checker = Checker::new().expect("checker creation should succeed");
        let invalid_defs = read_example("definitions/invalid_api.d.luau");
        let error = checker
            .add_definitions(&invalid_defs)
            .expect_err("invalid definitions should fail");

        let message = error.to_string();
        assert!(!message.trim().is_empty());
        assert!(message.contains("failed to load Luau definitions"));
    }

    /// Verifies custom definition labels are preserved in error messages.
    #[test]
    fn invalid_definitions_include_custom_label() {
        let mut checker = Checker::new().expect("checker creation should succeed");
        let invalid_defs = read_example("definitions/invalid_api.d.luau");
        let error = checker
            .add_definitions_with_name(&invalid_defs, "defs/invalid_api.d.luau")
            .expect_err("invalid definitions should fail");

        assert!(error.to_string().contains("defs/invalid_api.d.luau"));
    }

    /// Verifies multiple definitions with distinct labels stay active.
    #[test]
    fn multiple_definition_labels_keep_all_types_available() {
        let mut checker = Checker::new().expect("checker creation should succeed");
        checker
            .add_definitions_with_name("declare function alpha_id(): string", "defs/alpha.d.luau")
            .expect("alpha definitions should load");
        checker
            .add_definitions_with_name("declare function beta_count(): number", "defs/beta.d.luau")
            .expect("beta definitions should load");

        let result = checker
            .check(
                r#"
            --!strict
            local id: string = alpha_id()
            local count: number = beta_count()
            "#,
            )
            .unwrap();

        assert!(
            result.is_ok(),
            "both definition files should remain active: {result:#?}"
        );
    }

    /// Verifies host definitions affect check outcomes.
    #[test]
    fn definitions_change_check_behavior() {
        let mut checker = checker_with_demo_definitions();

        let ok_result = checker
            .check(
                r#"
            --!strict
            local todo = Todo.create():content("Review"):due("today"):save()
            todo:complete()
            "#,
            )
            .unwrap();
        assert!(
            ok_result.is_ok(),
            "expected script to pass with valid API usage: {ok_result:#?}"
        );

        let bad_result = checker
            .check(
                r#"
            --!strict
            local todo = Todo.create():content("Review"):due(42):save()
            "#,
            )
            .unwrap();
        assert!(!bad_result.is_ok(), "expected type error for due(42)");
    }

    /// Verifies one checker can run multiple checks while keeping definitions.
    #[test]
    fn checker_reuse_keeps_definitions() {
        let mut checker = checker_with_demo_definitions();

        let first = checker
            .check(
                r#"
            --!strict
            local _todo = Todo.create():content("one"):save()
            "#,
            )
            .unwrap();
        assert!(first.is_ok(), "first check should succeed");

        let second = checker
            .check(
                r#"
            --!strict
            local _todo = Todo.create():content("two"):due(123):save()
            "#,
            )
            .unwrap();
        assert!(!second.is_ok(), "second check should fail");

        let third = checker
            .check(
                r#"
            --!strict
            local id = make_id("todo")
            local _: string = id
            "#,
            )
            .unwrap();
        assert!(third.is_ok(), "third check should still succeed");
    }

    /// Verifies the checker remains stable when another Luau embedding is linked into the binary.
    #[test]
    fn checker_coexists_with_mlua() {
        let _lua = Lua::new();
        let mut checker = checker_with_demo_definitions();

        let result = checker
            .check(
                r#"
            --!strict
            local todo = Todo.create():content("Review"):save()
            todo:complete()
            "#,
            )
            .expect("check should run without internal compiler errors");

        assert!(result.is_ok(), "expected script to pass: {result:#?}");
    }

    /// Verifies the checker does not depend on the build-directory native library still existing.
    #[test]
    fn checker_works_without_build_directory_native_library() {
        let build_library_path = PathBuf::from(env!("LUAU_ANALYZE_NATIVE_LIB_PATH"));
        let backup_path = build_library_path.with_extension(format!(
            "{}.bak",
            build_library_path
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or("tmp")
        ));

        fs::rename(&build_library_path, &backup_path)
            .expect("should be able to temporarily hide the build-directory native library");
        let _restore = NativeLibraryRestore {
            original: build_library_path,
            backup: backup_path,
        };

        let mut checker = checker_with_demo_definitions();
        let result = checker
            .check(
                r#"
            --!strict
            local todo = Todo.create():content("Review"):save()
            todo:complete()
            "#,
            )
            .expect("check should not depend on the original build output path");

        assert!(result.is_ok(), "expected script to pass: {result:#?}");
    }

    /// Verifies empty source does not produce type errors.
    #[test]
    fn empty_script_is_ok() {
        let mut checker = Checker::new().expect("checker creation should succeed");
        let result = checker.check("").unwrap();
        assert!(result.is_ok(), "empty script should not produce errors");
    }

    /// Restores the build-directory native checker library after one test hides it.
    struct NativeLibraryRestore {
        /// Original build-output library path.
        original: PathBuf,
        /// Temporary backup path used during the test.
        backup: PathBuf,
    }

    impl Drop for NativeLibraryRestore {
        fn drop(&mut self) {
            drop(fs::rename(&self.backup, &self.original));
        }
    }

    /// Verifies syntax errors are surfaced as diagnostics.
    #[test]
    fn syntax_error_is_reported() {
        let mut checker = Checker::new().expect("checker creation should succeed");
        let result = checker
            .check(
                r#"
            --!strict
            local value: number =
            "#,
            )
            .unwrap();

        assert!(!result.is_ok(), "expected syntax error");
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| !diagnostic.message.is_empty())
        );
    }

    /// Verifies timeout state and labels are surfaced for zero-timeout checks.
    #[test]
    fn timeout_marks_result_and_uses_module_label() {
        let mut checker = Checker::new().expect("checker creation should succeed");
        let result = checker
            .check_with_options(
                "--!strict\nlocal x = 1\n",
                CheckOptions {
                    timeout: Some(Duration::ZERO),
                    module_name: Some("custom/module_timeout.luau"),
                    cancellation_token: None,
                    virtual_modules: &[],
                },
            )
            .unwrap();

        assert!(result.timed_out, "expected timeout marker");
        assert!(!result.is_ok(), "timeout should fail check");
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("custom/module_timeout.luau"))
        );
    }

    /// Verifies cancellation state is surfaced through check results.
    #[test]
    fn cancellation_marks_result() {
        let mut checker = Checker::new().expect("checker creation should succeed");
        let token = CancellationToken::new().expect("token should be created");
        token.cancel();

        let result = checker
            .check_with_options(
                "--!strict\nlocal x = 1\n",
                CheckOptions {
                    timeout: None,
                    module_name: Some("cancelled.luau"),
                    cancellation_token: Some(&token),
                    virtual_modules: &[],
                },
            )
            .unwrap();

        assert!(result.cancelled, "expected cancelled marker");
        assert!(!result.is_ok(), "cancelled check should fail");
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("cancelled"))
        );
    }

    /// Verifies bare source checks still need a filesystem root for relative requires.
    #[test]
    fn plain_source_check_without_filesystem_context_cannot_resolve_relative_require() {
        let mut checker = Checker::new().expect("checker creation should succeed");
        let result = checker
            .check(
                r#"
            --!strict
            local dep = require("./other_module")
            local _: number = dep.value
            "#,
            )
            .unwrap();

        assert!(
            !result.is_ok(),
            "expected unresolved module diagnostic without a filesystem root"
        );
    }

    /// Verifies `check_path` resolves adjacent filesystem modules.
    #[test]
    fn check_path_resolves_relative_filesystem_require() {
        let mut checker = Checker::new().expect("checker creation should succeed");
        let result = checker
            .check_path(&module_fixture("filesystem/requirer.luau"))
            .expect("filesystem graph should check");

        assert!(
            result.is_ok(),
            "filesystem require should resolve: {result:#?}"
        );
    }

    /// Verifies virtual modules can satisfy bare string `require(...)`.
    #[test]
    fn check_with_virtual_module_resolves_require() {
        let mut checker = Checker::new().expect("checker creation should succeed");
        let term = VirtualModule {
            name: "term",
            source: r#"
                export type Term = {
                    cols: number,
                }

                export type TermModule = {
                    current: () -> Term,
                }

                local module: TermModule = nil :: any
                return module
            "#,
        };
        let result = checker
            .check_with_options(
                r#"
                    --!strict
                    local term = require("term")
                    local _: number = term.current().cols
                "#,
                CheckOptions {
                    module_name: Some("virtual_root.luau"),
                    virtual_modules: &[term],
                    ..CheckOptions::default()
                },
            )
            .expect("virtual module graph should check");

        assert!(
            result.is_ok(),
            "virtual require should resolve: {result:#?}"
        );
    }

    /// Verifies one graph can mix filesystem and virtual require resolution.
    #[test]
    fn check_path_supports_mixed_filesystem_and_virtual_requires() {
        let mut checker = Checker::new().expect("checker creation should succeed");
        let term = VirtualModule {
            name: "term",
            source: r#"
                export type Term = {
                    cols: number,
                }

                export type TermModule = {
                    current: () -> Term,
                }

                local module: TermModule = nil :: any
                return module
            "#,
        };
        let result = checker
            .check_path_with_options(
                &module_fixture("mixed/requirer.luau"),
                CheckOptions {
                    virtual_modules: &[term],
                    ..CheckOptions::default()
                },
            )
            .expect("mixed graph should check");

        assert!(
            result.is_ok(),
            "mixed require graph should resolve: {result:#?}"
        );
    }

    /// Verifies diagnostics are deterministically sorted.
    #[test]
    fn diagnostics_are_sorted() {
        let mut checker = Checker::new().expect("checker creation should succeed");
        let result = checker
            .check(
                r#"
            --!strict
            local a: number = "x"
            local b: number = "y"
            "#,
            )
            .unwrap();

        for pair in result.diagnostics.windows(2) {
            let left = &pair[0];
            let right = &pair[1];
            let ordered = (left.line, left.col, left.severity, &left.message)
                <= (right.line, right.col, right.severity, &right.message);
            assert!(
                ordered,
                "diagnostics were not sorted: {left:?} then {right:?}"
            );
        }
    }

    /// Verifies all bundled example scripts match their declared expectations.
    #[test]
    fn bundled_examples_match_expectations() {
        let mut checker = checker_with_demo_definitions();
        let scripts_dir = examples_root().join("scripts");
        let mut scripts = collect_scripts_recursive(&scripts_dir)
            .expect("scripts should be collected recursively");
        scripts.sort();

        let mut mismatches = Vec::new();
        for script in scripts {
            let source = fs::read_to_string(&script).expect("example script should be readable");
            let expected = parse_expectation(&source);
            let result = checker.check(&source).unwrap();
            let actual = if result.is_ok() {
                Expectation::Pass
            } else {
                Expectation::Fail
            };
            if actual != expected {
                mismatches.push(format!(
                    "{} expected {:?} got {:?}",
                    script.display(),
                    expected,
                    actual
                ));
            }
        }

        assert!(
            mismatches.is_empty(),
            "script expectation mismatches:\n{}",
            mismatches.join("\n")
        );
    }

    /// Verifies packaged fixtures stay in sync with the workspace examples.
    #[test]
    fn bundled_examples_match_workspace_examples() {
        let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        if !workspace_root.exists() {
            return;
        }

        let bundled_root = examples_root();
        let mut bundled_files = collect_scripts_recursive(&bundled_root)
            .expect("bundled examples should be collected recursively")
            .into_iter()
            .map(|path| {
                path.strip_prefix(&bundled_root)
                    .expect("bundled file should stay under bundled root")
                    .to_path_buf()
            })
            .collect::<Vec<_>>();
        let mut workspace_files = collect_scripts_recursive(&workspace_root)
            .expect("workspace examples should be collected recursively")
            .into_iter()
            .map(|path| {
                path.strip_prefix(&workspace_root)
                    .expect("workspace file should stay under workspace root")
                    .to_path_buf()
            })
            .collect::<Vec<_>>();

        bundled_files.sort();
        workspace_files.sort();

        assert_eq!(
            bundled_files, workspace_files,
            "bundled fixtures drifted from workspace examples"
        );

        for relative_path in bundled_files {
            let bundled = fs::read_to_string(bundled_root.join(&relative_path))
                .expect("bundled example should be readable");
            let workspace = fs::read_to_string(workspace_root.join(&relative_path))
                .expect("workspace example should be readable");
            assert_eq!(
                bundled,
                workspace,
                "bundled fixture `{}` drifted from workspace example",
                relative_path.display()
            );
        }
    }

    /// Creates a checker preloaded with demo API definitions.
    fn checker_with_demo_definitions() -> Checker {
        let mut checker = Checker::new().expect("checker creation should succeed");
        let defs = read_example("definitions/api.d.luau");
        checker
            .add_definitions(&defs)
            .expect("demo definitions should load");
        checker
    }

    /// Reads one file under the crate-bundled examples fixture directory.
    fn read_example(relative_path: &str) -> String {
        let path = examples_root().join(relative_path);
        fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("failed to read `{}`: {error}", path.display());
        })
    }

    /// Returns the crate-bundled examples fixture root.
    fn examples_root() -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/examples");
        assert!(
            root.exists(),
            "examples root should exist at `{}`",
            root.display()
        );
        root
    }

    /// Returns one checked-in module fixture path.
    fn module_fixture(relative_path: &str) -> PathBuf {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/modules")
            .join(relative_path);
        assert!(
            path.exists(),
            "module fixture should exist at `{}`",
            path.display()
        );
        path
    }

    /// Parses the script expectation marker from leading comments.
    fn parse_expectation(source: &str) -> Expectation {
        for line in source.lines().take(10) {
            let normalized = line.trim();
            if let Some(rest) = normalized.strip_prefix("-- expect:") {
                let marker = rest.trim();
                if marker.eq_ignore_ascii_case("fail") || marker.eq_ignore_ascii_case("error") {
                    return Expectation::Fail;
                }
                if marker.eq_ignore_ascii_case("pass") || marker.eq_ignore_ascii_case("ok") {
                    return Expectation::Pass;
                }
            }
            if !normalized.is_empty() && !normalized.starts_with("--") {
                break;
            }
        }
        Expectation::Pass
    }

    /// Recursively collects all `.luau` scripts under `root`.
    fn collect_scripts_recursive(root: &Path) -> Result<Vec<PathBuf>, String> {
        let mut scripts = Vec::new();
        let mut stack = vec![root.to_path_buf()];

        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).map_err(|error| {
                format!("failed to read scripts dir `{}`: {error}", dir.display())
            })? {
                let entry =
                    entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|ext| ext == "luau") {
                    scripts.push(path);
                }
            }
        }

        Ok(scripts)
    }
}
