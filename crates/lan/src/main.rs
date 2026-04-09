//! Command-line utility for `luau-analyze` checks.

use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

use clap::{Args, Parser, Subcommand};
use luau_analyze::{
    CancellationToken, CheckOptions, CheckResult, Checker, Severity, checker_policy,
};
use serde::Serialize;

/// Default definitions file used by `--default-definitions`.
const DEFAULT_DEFINITIONS: &str = "examples/definitions/api.d.luau";

/// Top-level CLI arguments.
#[derive(Debug, Parser)]
#[command(about = "Luau analysis CLI")]
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
    /// Also load the repository default definitions file.
    #[arg(long)]
    default_definitions: bool,
    /// Treat warnings as a non-zero exit status.
    #[arg(long)]
    fail_on_warnings: bool,
    /// Per-check timeout in milliseconds.
    #[arg(long)]
    timeout_ms: Option<u64>,
    /// Print strict checker policy and exit.
    #[arg(long)]
    print_policy: bool,
    /// Emit machine-readable JSON output.
    #[arg(long)]
    json: bool,
    /// Cancel the check before execution (realtime interruption demo).
    #[arg(long)]
    cancel_immediately: bool,
}

/// JSON diagnostic representation.
#[derive(Debug, Serialize)]
struct JsonDiagnostic {
    /// One-based start line.
    line: u32,
    /// One-based start column.
    col: u32,
    /// One-based end line.
    end_line: u32,
    /// One-based end column.
    end_col: u32,
    /// Severity as lowercase string.
    severity: &'static str,
    /// Diagnostic message.
    message: String,
}

/// JSON output for one check run.
#[derive(Debug, Serialize)]
struct JsonCheckOutput {
    /// Input label.
    target: String,
    /// Whether the check produced no errors.
    ok: bool,
    /// Static checker policy name.
    solver: &'static str,
    /// Strict mode policy value.
    strict_mode: bool,
    /// Whether this run timed out.
    timed_out: bool,
    /// Whether cancellation was requested.
    cancelled: bool,
    /// Diagnostics for this run.
    diagnostics: Vec<JsonDiagnostic>,
}

/// Entrypoint for the CLI binary.
fn main() -> ExitCode {
    let args = CliArgs::parse();
    let result = match args.command {
        LanCommand::Check(check) => run_check(&check),
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
    if args.print_policy {
        print_policy(args.json)?;
        return Ok(ExitCode::SUCCESS);
    }

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

    let label = args
        .file
        .as_deref()
        .map_or_else(|| "<stdin>".to_owned(), |path| path.display().to_string());

    let cancellation_token = if args.cancel_immediately {
        let token = CancellationToken::new().map_err(|error| error.to_string())?;
        token.cancel();
        Some(token)
    } else {
        None
    };

    let result = checker
        .check_with_options(
            &source,
            CheckOptions {
                timeout: args.timeout_ms.map(Duration::from_millis),
                module_name: Some(label.as_str()),
                cancellation_token: cancellation_token.as_ref(),
            },
        )
        .map_err(|error| error.to_string())?;

    if args.json {
        let output = JsonCheckOutput {
            target: label.clone(),
            ok: result.is_ok(),
            solver: checker_policy().solver,
            strict_mode: checker_policy().strict_mode,
            timed_out: result.timed_out,
            cancelled: result.cancelled,
            diagnostics: diagnostics_to_json(&result),
        };
        print_json(&output)?;
    } else {
        print_diagnostics(&label, &result);
    }

    let has_errors = !result.is_ok();
    let has_warnings = result.has_warnings();
    if has_errors || (args.fail_on_warnings && has_warnings) {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

/// Prints fixed checker policy as text or JSON.
fn print_policy(as_json: bool) -> Result<(), String> {
    let policy = checker_policy();

    if as_json {
        #[derive(Debug, Serialize)]
        struct PolicyJson {
            strict_mode: bool,
            solver: &'static str,
            exposes_batch_queue: bool,
        }

        print_json(&PolicyJson {
            strict_mode: policy.strict_mode,
            solver: policy.solver,
            exposes_batch_queue: policy.exposes_batch_queue,
        })
    } else {
        println!("strict_mode={}", policy.strict_mode);
        println!("solver={}", policy.solver);
        println!("exposes_batch_queue={}", policy.exposes_batch_queue);
        Ok(())
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

/// Reads all bytes from stdin and decodes UTF-8 text.
fn read_stdin() -> Result<String, String> {
    let mut buffer = Vec::new();
    io::stdin()
        .read_to_end(&mut buffer)
        .map_err(|error| format!("failed to read stdin: {error}"))?;
    String::from_utf8(buffer).map_err(|error| format!("stdin is not valid UTF-8: {error}"))
}

/// Converts diagnostics to JSON rows.
fn diagnostics_to_json(result: &CheckResult) -> Vec<JsonDiagnostic> {
    result
        .diagnostics
        .iter()
        .map(|diagnostic| JsonDiagnostic {
            line: diagnostic.line + 1,
            col: diagnostic.col + 1,
            end_line: diagnostic.end_line + 1,
            end_col: diagnostic.end_col + 1,
            severity: match diagnostic.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            },
            message: diagnostic.message.clone(),
        })
        .collect()
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

/// Prints any serializable value as pretty JSON.
fn print_json<T: Serialize>(value: &T) -> Result<(), String> {
    let json = serde_json::to_string_pretty(value)
        .map_err(|error| format!("failed to serialize JSON output: {error}"))?;
    println!("{json}");
    Ok(())
}
