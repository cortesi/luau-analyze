//! Integration tests for the `lan` command-line interface.

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        path::{Path, PathBuf},
        process::{Command, Output},
        time::{SystemTime, UNIX_EPOCH},
    };

    use serde_json::Value;

    /// Returns the absolute path to the compiled `lan` binary.
    fn lan_bin() -> PathBuf {
        PathBuf::from(env!("CARGO_BIN_EXE_lan"))
    }

    /// Returns the workspace root for fixture resolution.
    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("workspace root should resolve")
    }

    /// Executes `lan` with arguments from the workspace root.
    fn run_lan(args: &[&str]) -> Output {
        Command::new(lan_bin())
            .args(args)
            .current_dir(workspace_root())
            .output()
            .expect("lan command should start")
    }

    /// Parses UTF-8 stdout as JSON value.
    fn parse_stdout_json(output: &Output) -> Value {
        let text = String::from_utf8(output.stdout.clone()).expect("stdout should be valid UTF-8");
        serde_json::from_str(&text).expect("stdout should be valid JSON")
    }

    /// Verifies JSON output includes strict policy fields for a successful check.
    #[test]
    fn check_json_reports_policy_for_success() {
        let output = run_lan(&[
            "check",
            "--json",
            "--default-definitions",
            "examples/scripts/01_ok_builder.luau",
        ]);

        assert!(output.status.success(), "command should succeed");
        let value = parse_stdout_json(&output);
        assert_eq!(value["ok"], true);
        assert_eq!(value["strict_mode"], true);
        assert_eq!(value["solver"], "new");
        assert_eq!(value["timed_out"], false);
        assert_eq!(value["cancelled"], false);
    }

    /// Verifies timeout state is surfaced in JSON output.
    #[test]
    fn check_json_reports_timeout() {
        let output = run_lan(&[
            "check",
            "--json",
            "--default-definitions",
            "--timeout-ms",
            "0",
            "examples/scripts/01_ok_builder.luau",
        ]);

        assert!(
            !output.status.success(),
            "timeout should produce non-zero exit"
        );
        let value = parse_stdout_json(&output);
        assert_eq!(value["ok"], false);
        assert_eq!(value["timed_out"], true);
        assert_eq!(value["cancelled"], false);
    }

    /// Verifies cancellation state is surfaced in JSON output.
    #[test]
    fn check_json_reports_cancellation() {
        let output = run_lan(&[
            "check",
            "--json",
            "--default-definitions",
            "--cancel-immediately",
            "examples/scripts/01_ok_builder.luau",
        ]);

        assert!(
            !output.status.success(),
            "cancelled run should produce non-zero exit"
        );
        let value = parse_stdout_json(&output);
        assert_eq!(value["ok"], false);
        assert_eq!(value["timed_out"], false);
        assert_eq!(value["cancelled"], true);
    }

    /// Verifies definition-load failures include the specific failing file path.
    #[test]
    fn check_definition_errors_include_failing_path() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let root = env::temp_dir().join(format!("lan-definitions-{unique}"));
        fs::create_dir_all(&root).expect("temp directory should be created");

        let valid = root.join("good.d.luau");
        let invalid = root.join("bad.d.luau");
        fs::write(&valid, "declare function good_api(): string\n")
            .expect("valid definitions should be written");
        fs::write(
            &invalid,
            "declare class Broken\n    function missing_end(self): ()\n",
        )
        .expect("invalid definitions should be written");

        let valid_arg = valid.to_string_lossy().into_owned();
        let invalid_arg = invalid.to_string_lossy().into_owned();

        let output = run_lan(&[
            "check",
            "-d",
            valid_arg.as_str(),
            "-d",
            invalid_arg.as_str(),
            "examples/scripts/06_ok_type_refinement.luau",
        ]);

        assert!(
            !output.status.success(),
            "invalid definitions should fail command"
        );
        let stderr = String::from_utf8(output.stderr).expect("stderr should be valid UTF-8");
        assert!(
            stderr.contains(&invalid_arg),
            "stderr should include failing path"
        );

        fs::remove_dir_all(&root).expect("temp directory should be cleaned up");
    }
}
