//! Integration tests for `luau-analyze`.

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    use luau_analyze::{Checker, Severity};

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
        let result = checker.check(
            r#"
            --!strict
            local x: number = "hello"
            "#,
        );

        assert!(!result.is_ok(), "expected strict type mismatch");
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

    /// Verifies host definitions affect check outcomes.
    #[test]
    fn definitions_change_check_behavior() {
        let mut checker = checker_with_demo_definitions();

        let ok_result = checker.check(
            r#"
            --!strict
            local todo = Todo.create():content("Review"):due("today"):save()
            todo:complete()
            "#,
        );
        assert!(
            ok_result.is_ok(),
            "expected script to pass with valid API usage: {ok_result:#?}"
        );

        let bad_result = checker.check(
            r#"
            --!strict
            local todo = Todo.create():content("Review"):due(42):save()
            "#,
        );
        assert!(!bad_result.is_ok(), "expected type error for due(42)");
    }

    /// Verifies one checker can run multiple checks while keeping definitions.
    #[test]
    fn checker_reuse_keeps_definitions() {
        let mut checker = checker_with_demo_definitions();

        let first = checker.check(
            r#"
            --!strict
            local _todo = Todo.create():content("one"):save()
            "#,
        );
        assert!(first.is_ok(), "first check should succeed");

        let second = checker.check(
            r#"
            --!strict
            local _todo = Todo.create():content("two"):due(123):save()
            "#,
        );
        assert!(!second.is_ok(), "second check should fail");

        let third = checker.check(
            r#"
            --!strict
            local id = make_id("todo")
            local _: string = id
            "#,
        );
        assert!(third.is_ok(), "third check should still succeed");
    }

    /// Verifies empty source does not produce type errors.
    #[test]
    fn empty_script_is_ok() {
        let mut checker = Checker::new().expect("checker creation should succeed");
        let result = checker.check("");
        assert!(result.is_ok(), "empty script should not produce errors");
    }

    /// Verifies syntax errors are surfaced as diagnostics.
    #[test]
    fn syntax_error_is_reported() {
        let mut checker = Checker::new().expect("checker creation should succeed");
        let result = checker.check(
            r#"
            --!strict
            local value: number =
            "#,
        );

        assert!(!result.is_ok(), "expected syntax error");
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| !diagnostic.message.is_empty())
        );
    }

    /// Verifies diagnostics are deterministically sorted.
    #[test]
    fn diagnostics_are_sorted() {
        let mut checker = Checker::new().expect("checker creation should succeed");
        let result = checker.check(
            r#"
            --!strict
            local a: number = "x"
            local b: number = "y"
            "#,
        );

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
        let mut scripts = fs::read_dir(&scripts_dir)
            .expect("scripts directory should exist")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "luau"))
            .collect::<Vec<_>>();
        scripts.sort();

        let mut mismatches = Vec::new();
        for script in scripts {
            let source = fs::read_to_string(&script).expect("example script should be readable");
            let expected = parse_expectation(&source);
            let result = checker.check(&source);
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

    /// Creates a checker preloaded with demo API definitions.
    fn checker_with_demo_definitions() -> Checker {
        let mut checker = Checker::new().expect("checker creation should succeed");
        let defs = read_example("definitions/api.d.luau");
        checker
            .add_definitions(&defs)
            .expect("demo definitions should load");
        checker
    }

    /// Reads one file under the repository-level `examples/` directory.
    fn read_example(relative_path: &str) -> String {
        let path = examples_root().join(relative_path);
        fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("failed to read `{}`: {error}", path.display());
        })
    }

    /// Returns the repository-level `examples/` root.
    fn examples_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .canonicalize()
            .expect("examples root should exist")
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
}
