//! Command-line utility for `luau-analyze`.

use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Args, Parser, Subcommand};
use luau_analyze::{CheckResult, Checker, Severity};

/// Default definitions file used by `lan demo`.
const DEFAULT_DEFINITIONS: &str = "examples/definitions/api.d.luau";
/// Default script directory used by `lan demo`.
const DEFAULT_SCRIPTS_DIR: &str = "examples/scripts";

/// Top-level CLI arguments.
#[derive(Debug, Parser)]
#[command(about = "Luau analysis demo CLI")]
struct CliArgs {
    /// Selected command.
    #[command(subcommand)]
    command: LanCommand,
}

/// Supported `lan` subcommands.
#[derive(Debug, Subcommand)]
enum LanCommand {
    /// Check a Luau source file (or stdin) and print diagnostics.
    Check(CheckArgs),
    /// Run all bundled examples and validate expected outcomes.
    Demo(DemoArgs),
}

/// Arguments for `lan check`.
#[derive(Debug, Args)]
struct CheckArgs {
    /// Optional path to source file; if omitted, source is read from stdin.
    #[arg(value_name = "FILE")]
    file: Option<PathBuf>,
    /// One or more Luau definition files to load before checking.
    #[arg(short = 'd', long = "definitions", value_name = "FILE")]
    definitions: Vec<PathBuf>,
    /// Also load the repository demo definitions file.
    #[arg(long)]
    default_definitions: bool,
    /// Treat warnings as a non-zero exit status.
    #[arg(long)]
    fail_on_warnings: bool,
}

/// Arguments for `lan demo`.
#[derive(Debug, Args)]
struct DemoArgs {
    /// Definitions file used for all scripts in the demo run.
    #[arg(long, default_value = DEFAULT_DEFINITIONS)]
    definitions: PathBuf,
    /// Directory containing `.luau` script examples.
    #[arg(long, default_value = DEFAULT_SCRIPTS_DIR)]
    scripts_dir: PathBuf,
    /// Treat warnings as a failed run for scripts expected to pass.
    #[arg(long)]
    fail_on_warnings: bool,
}

/// Expected outcome parsed from a script header comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expectation {
    /// Script is expected to type-check without errors.
    Pass,
    /// Script is expected to produce at least one error.
    Fail,
}

/// Entrypoint for the CLI binary.
fn main() -> ExitCode {
    let args = CliArgs::parse();
    let result = match args.command {
        LanCommand::Check(check) => run_check(&check),
        LanCommand::Demo(demo) => run_demo(&demo),
    };

    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("lan: {error}");
            ExitCode::from(2)
        }
    }
}

/// Executes the `check` subcommand.
fn run_check(args: &CheckArgs) -> Result<ExitCode, String> {
    let mut checker = Checker::new().map_err(|error| error.to_string())?;
    if args.default_definitions {
        load_definitions_file(&mut checker, Path::new(DEFAULT_DEFINITIONS))?;
    }
    for definitions in &args.definitions {
        load_definitions_file(&mut checker, definitions)?;
    }

    let source = match &args.file {
        Some(path) => fs::read_to_string(path)
            .map_err(|error| format!("failed to read `{}`: {error}", path.display()))?,
        None => read_stdin()?,
    };

    let result = checker.check(&source);
    let label = args
        .file
        .as_deref()
        .map_or_else(|| "<stdin>".to_owned(), |path| path.display().to_string());
    print_diagnostics(&label, &result);

    let has_errors = !result.is_ok();
    let has_warnings = !result.warnings().is_empty();
    if has_errors || (args.fail_on_warnings && has_warnings) {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

/// Executes the `demo` subcommand.
fn run_demo(args: &DemoArgs) -> Result<ExitCode, String> {
    let mut checker = Checker::new().map_err(|error| error.to_string())?;
    load_definitions_file(&mut checker, &args.definitions)?;

    let mut scripts = collect_scripts(&args.scripts_dir)?;
    if scripts.is_empty() {
        return Err(format!(
            "no `.luau` scripts found under `{}`",
            args.scripts_dir.display()
        ));
    }
    scripts.sort();

    let mut failed = 0usize;
    for script in scripts {
        let source = fs::read_to_string(&script)
            .map_err(|error| format!("failed to read `{}`: {error}", script.display()))?;
        let expectation = parse_expectation(&source);
        let result = checker.check(&source);
        let has_errors = !result.is_ok();
        let has_warnings = !result.warnings().is_empty();
        let passed = match expectation {
            Expectation::Pass => !has_errors && (!args.fail_on_warnings || !has_warnings),
            Expectation::Fail => has_errors,
        };

        let tag = if passed { "PASS" } else { "FAIL" };
        println!("{tag} {}", script.display());
        print_diagnostics(&script.display().to_string(), &result);
        if !passed {
            failed += 1;
        }
    }

    if failed == 0 {
        println!("demo summary: all scripts matched expectations");
        Ok(ExitCode::SUCCESS)
    } else {
        println!("demo summary: {failed} script(s) did not match expectations");
        Ok(ExitCode::from(1))
    }
}

/// Loads one definitions file into a checker.
fn load_definitions_file(checker: &mut Checker, path: &Path) -> Result<(), String> {
    let definitions = fs::read_to_string(path)
        .map_err(|error| format!("failed to read definitions `{}`: {error}", path.display()))?;
    checker
        .add_definitions(&definitions)
        .map_err(|error| error.to_string())
}

/// Reads all bytes from stdin and decodes UTF-8 text.
fn read_stdin() -> Result<String, String> {
    let mut buffer = Vec::new();
    io::stdin()
        .read_to_end(&mut buffer)
        .map_err(|error| format!("failed to read stdin: {error}"))?;
    String::from_utf8(buffer).map_err(|error| format!("stdin is not valid UTF-8: {error}"))
}

/// Collects all `.luau` scripts under a directory.
fn collect_scripts(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut scripts = Vec::new();
    for entry in fs::read_dir(root)
        .map_err(|error| format!("failed to read scripts dir `{}`: {error}", root.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read directory entry: {error}"))?;
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "luau")
        {
            scripts.push(path);
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

/// Prints diagnostics in a simple `file:line:col` format.
fn print_diagnostics(label: &str, result: &CheckResult) {
    for diagnostic in &result.diagnostics {
        let severity = match diagnostic.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        println!(
            "{label}:{}:{}: {severity}: {}",
            diagnostic.line + 1,
            diagnostic.col + 1,
            diagnostic.message
        );
    }
}
