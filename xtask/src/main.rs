//! Project-local task runner.

use std::{
    env, fs,
    io::{IsTerminal, stdout},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    time::Duration,
};

use clap::{Args, Parser, Subcommand};
use luau_analyze::{CheckOptions, Checker, Severity};

/// Default definitions file used by smoke checks.
const DEFAULT_DEFINITIONS: &str = "examples/definitions/api.d.luau";
/// Default scripts directory used by smoke checks.
const DEFAULT_SCRIPTS_DIR: &str = "examples/scripts";
/// Relative path from workspace root to the Luau submodule.
const LUAU_SUBMODULE: &str = "crates/luau-analyze/luau";

/// Top-level command-line arguments.
#[derive(Debug, Parser)]
#[command(about = "Project task runner")]
struct CliArgs {
    /// Selected xtask subcommand.
    #[command(subcommand)]
    command: XtaskCommand,
}

/// Supported xtask subcommands.
#[derive(Debug, Subcommand)]
enum XtaskCommand {
    /// Format and lint the workspace.
    Tidy,
    /// Run the workspace test suite.
    Test,
    /// Run example Luau smoke checks.
    Smoke(SmokeArgs),
    /// Update the Luau submodule to the latest upstream tag.
    LuauUpdate,
}

/// Arguments for `xtask smoke`.
#[derive(Debug, Args)]
struct SmokeArgs {
    /// Definitions file used for all scripts in the smoke run.
    #[arg(long, default_value = DEFAULT_DEFINITIONS)]
    definitions: PathBuf,
    /// Directory containing `.luau` scripts for smoke checks.
    #[arg(long, default_value = DEFAULT_SCRIPTS_DIR)]
    scripts_dir: PathBuf,
    /// Treat warnings as failures for scripts expected to pass.
    #[arg(long)]
    fail_on_warnings: bool,
    /// Per-script timeout in milliseconds.
    #[arg(long)]
    timeout_ms: Option<u64>,
}

/// Expected outcome parsed from a script header comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expectation {
    /// Script is expected to type-check without errors.
    Pass,
    /// Script is expected to produce at least one error.
    Fail,
}

/// ANSI color helper.
#[derive(Debug, Clone, Copy)]
struct Colors {
    /// Whether ANSI color output is enabled.
    enabled: bool,
}

impl Colors {
    /// Creates a new color helper based on terminal capability and env flags.
    fn detect() -> Self {
        let enabled = if env::var_os("NO_COLOR").is_some() {
            false
        } else if env::var_os("CLICOLOR_FORCE").is_some() {
            true
        } else {
            stdout().is_terminal()
        };
        Self { enabled }
    }

    /// Applies an ANSI color code when enabled.
    fn paint(self, text: impl AsRef<str>, code: &str) -> String {
        let text = text.as_ref();
        if self.enabled {
            format!("\u{1b}[{code}m{text}\u{1b}[0m")
        } else {
            text.to_owned()
        }
    }
}

/// Parses arguments and dispatches to the selected subcommand.
fn main() -> ExitCode {
    let args = CliArgs::parse();
    let result = match args.command {
        XtaskCommand::Tidy => tidy(),
        XtaskCommand::Test => test(),
        XtaskCommand::Smoke(smoke) => smoke_check(&smoke),
        XtaskCommand::LuauUpdate => luau_update(),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

/// Formats and lints the workspace.
fn tidy() -> Result<(), String> {
    run_cargo(&[
        "+nightly",
        "fmt",
        "--all",
        "--",
        "--config-path",
        "./rustfmt-nightly.toml",
    ])?;
    run_cargo(&[
        "clippy",
        "-q",
        "--fix",
        "--all",
        "--all-targets",
        "--all-features",
        "--allow-dirty",
        "--tests",
        "--examples",
    ])?;
    Ok(())
}

/// Runs the workspace tests through `cargo-nextest`.
fn test() -> Result<(), String> {
    run_cargo(&["nextest", "run", "--all"])
}

/// Updates the Luau submodule to the latest upstream tag.
fn luau_update() -> Result<(), String> {
    let workspace = workspace_root();
    let submodule = workspace.join(LUAU_SUBMODULE);
    let colors = Colors::detect();

    println!("{}", colors.paint("Fetching tags from upstream...", "1"));
    run_git(&submodule, &["fetch", "--tags"])?;

    let current = run_git(&submodule, &["describe", "--tags", "--exact-match", "HEAD"])
        .unwrap_or_else(|_| "(unknown)".to_owned());
    let all_tags = run_git(&submodule, &["tag"])?;
    let latest = latest_luau_tag(&all_tags)?;

    println!("  current: {}", colors.paint(&current, "33;1"));
    println!("   latest: {}", colors.paint(&latest, "36;1"));

    if current == latest {
        println!("{}", colors.paint("Already up to date.", "32;1"));
        return Ok(());
    }

    println!(
        "\n{}",
        colors.paint(format!("Checking out {latest}..."), "1")
    );
    run_git(&submodule, &["checkout", &latest])?;

    println!("{}", colors.paint("Cleaning build artifacts...", "1"));
    run_cargo(&["clean", "-p", "luau-analyze"])?;

    println!("{}", colors.paint("Building...", "1"));
    run_cargo(&["build", "-p", "luau-analyze"])?;

    println!("{}", colors.paint("Running tests...", "1"));
    run_cargo(&["nextest", "run", "--workspace"])?;

    println!("\n{}", colors.paint("Update successful.", "32;1"));
    println!("To commit:");
    println!("  git add {LUAU_SUBMODULE}");
    println!("  git commit -m \"chore: update Luau to {latest}\"");

    Ok(())
}

/// Runs smoke checks for all bundled example scripts.
fn smoke_check(args: &SmokeArgs) -> Result<(), String> {
    let workspace = workspace_root();
    let definitions_path = absolute_or_workspace(&workspace, &args.definitions);
    let scripts_root = absolute_or_workspace(&workspace, &args.scripts_dir);

    let mut checker = Checker::new().map_err(|error| error.to_string())?;
    load_definitions_file(&mut checker, &definitions_path)?;

    let mut scripts = collect_scripts_recursive(&scripts_root)?;
    if scripts.is_empty() {
        return Err(format!(
            "no `.luau` scripts found under `{}`",
            scripts_root.display()
        ));
    }
    scripts.sort();

    let colors = Colors::detect();
    let mut failed = 0usize;

    for script in &scripts {
        let source = fs::read_to_string(script)
            .map_err(|error| format!("failed to read `{}`: {error}", script.display()))?;
        let expectation = parse_expectation(&source);
        let label = display_path(script, &workspace);

        let result = checker.check_with_options(
            &source,
            CheckOptions {
                timeout: args.timeout_ms.map(Duration::from_millis),
                module_name: Some(label.as_str()),
                cancellation_token: None,
            },
        );

        let (result, check_error) = match result {
            Ok(res) => (Some(res), None),
            Err(e) => (None, Some(e)),
        };

        let has_errors = result.as_ref().is_none_or(|r| !r.is_ok());
        let has_warnings = result.as_ref().is_some_and(|result| result.has_warnings());
        let passed = match expectation {
            Expectation::Pass => !has_errors && (!args.fail_on_warnings || !has_warnings),
            Expectation::Fail => has_errors,
        };

        let tag = if passed {
            colors.paint("PASS", "32;1")
        } else {
            colors.paint("FAIL", "31;1")
        };
        println!("{tag} {label}");

        if let Some(err) = check_error {
            println!("  {} {}", colors.paint("error", "31"), err);
        }

        if let Some(result) = &result {
            for diagnostic in &result.diagnostics {
                let severity = match diagnostic.severity {
                    Severity::Error => colors.paint("error", "31"),
                    Severity::Warning => colors.paint("warning", "33"),
                };
                println!(
                    "  {severity} {}:{} {}",
                    diagnostic.line + 1,
                    diagnostic.col + 1,
                    diagnostic.message
                );
            }

            if result.timed_out {
                println!("  {}", colors.paint("timeout", "31"));
            }
            if result.cancelled {
                println!("  {}", colors.paint("cancelled", "33"));
            }
        }

        if !passed {
            failed += 1;
        }
    }

    if failed == 0 {
        println!(
            "{}",
            colors.paint(
                format!(
                    "smoke summary: all {} scripts matched expectations",
                    scripts.len()
                ),
                "32;1",
            )
        );
        Ok(())
    } else {
        println!(
            "{}",
            colors.paint(
                format!(
                    "smoke summary: {failed}/{} scripts did not match expectations",
                    scripts.len()
                ),
                "31;1",
            )
        );
        Err(format!("{failed} smoke script(s) failed"))
    }
}

/// Loads one definitions file into a checker.
fn load_definitions_file(checker: &mut Checker, path: &Path) -> Result<(), String> {
    let definitions = fs::read_to_string(path)
        .map_err(|error| format!("failed to read definitions `{}`: {error}", path.display()))?;

    checker
        .add_definitions_with_name(&definitions, &path.display().to_string())
        .map_err(|error| format!("{}: {error}", path.display()))
}

/// Returns `path` as absolute, resolving relative paths against workspace root.
fn absolute_or_workspace(workspace: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.join(path)
    }
}

/// Formats `path` relative to workspace root when possible.
fn display_path(path: &Path, workspace: &Path) -> String {
    path.strip_prefix(workspace)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Recursively collects all `.luau` scripts under a directory.
fn collect_scripts_recursive(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut scripts = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)
            .map_err(|error| format!("failed to read scripts dir `{}`: {error}", dir.display()))?
        {
            let entry =
                entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
            let path = entry.path();

            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "luau")
            {
                scripts.push(path);
            }
        }
    }

    Ok(scripts)
}

/// Parses `-- expect: ...` from script header comments.
fn parse_expectation(source: &str) -> Expectation {
    for line in source.lines().take(10) {
        let normalized = line.trim();
        if let Some(rest) = normalized.strip_prefix("-- expect:") {
            let tag = rest.trim();
            if tag.eq_ignore_ascii_case("fail") || tag.eq_ignore_ascii_case("error") {
                return Expectation::Fail;
            }
            if tag.eq_ignore_ascii_case("pass") || tag.eq_ignore_ascii_case("ok") {
                return Expectation::Pass;
            }
        }
        if !normalized.is_empty() && !normalized.starts_with("--") {
            break;
        }
    }
    Expectation::Pass
}

/// Runs a git command in the given directory and captures stdout.
fn run_git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|error| format!("failed to run `git {}`: {error}", args.join(" ")))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "`git {}` failed: {}",
            args.join(" "),
            stderr.trim()
        ))
    }
}

/// Returns the highest `0.NNN` tag from a newline-separated tag list.
fn latest_luau_tag(tag_output: &str) -> Result<String, String> {
    tag_output
        .lines()
        .filter_map(|tag| {
            tag.strip_prefix("0.")
                .and_then(|n| n.parse::<u64>().ok())
                .map(|n| (n, tag))
        })
        .max_by_key(|(n, _)| *n)
        .map(|(_, tag)| tag.to_owned())
        .ok_or_else(|| "no valid Luau tags (0.NNN) found".to_owned())
}

/// Executes `cargo` in the workspace root.
fn run_cargo(args: &[&str]) -> Result<(), String> {
    let workspace_root = workspace_root();
    let status = Command::new("cargo")
        .args(args)
        .current_dir(&workspace_root)
        .status()
        .map_err(|error| format!("failed to run `cargo {}`: {error}", args.join(" ")))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "`cargo {}` failed with status {status}",
            args.join(" ")
        ))
    }
}

/// Resolves the workspace root from the `xtask` crate directory.
fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .map_or_else(|| manifest_dir.clone(), PathBuf::from)
}

/// Tests for smoke-script helpers.
#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{Expectation, collect_scripts_recursive, latest_luau_tag, parse_expectation};

    /// Verifies expectation parsing for pass markers.
    #[test]
    fn parse_expectation_pass_marker() {
        let source = "-- expect: pass\n--!strict\nlocal x = 1\n";
        assert_eq!(Expectation::Pass, parse_expectation(source));
    }

    /// Verifies expectation parsing for fail markers.
    #[test]
    fn parse_expectation_fail_marker() {
        let source = "-- expect: fail\n--!strict\nlocal x: number = \"x\"\n";
        assert_eq!(Expectation::Fail, parse_expectation(source));
    }

    /// Verifies tag selection picks the highest `0.NNN` version.
    #[test]
    fn latest_luau_tag_picks_highest() {
        let tags = "0.708\n0.710\n0.709\n696\n0.700\n";
        assert_eq!("0.710", latest_luau_tag(tags).unwrap());
    }

    /// Verifies non-prefixed tags are rejected.
    #[test]
    fn latest_luau_tag_rejects_non_prefixed() {
        assert!(latest_luau_tag("696\n").is_err());
    }

    /// Verifies recursive script discovery includes nested files.
    #[test]
    fn recursive_collection_finds_nested_luau_files() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        let root = env::temp_dir().join(format!("xtask-recursive-{unique}"));
        let nested = root.join("nested").join("deep");
        fs::create_dir_all(&nested).expect("temp directory should be created");

        let top_file = root.join("top.luau");
        let nested_file = nested.join("deep.luau");
        fs::write(&top_file, "--!strict\n").expect("top script should be written");
        fs::write(&nested_file, "--!strict\n").expect("nested script should be written");

        let mut scripts = collect_scripts_recursive(&root).expect("script collection should work");
        scripts.sort();

        assert_eq!(2, scripts.len());
        assert!(scripts.contains(&PathBuf::from(&top_file)));
        assert!(scripts.contains(&PathBuf::from(&nested_file)));

        fs::remove_dir_all(&root).expect("temp directory should be cleaned up");
    }
}
