//! Command-line utility for `luau-analyze` checks.

use std::{
    io::{self, Read},
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

use clap::{Args, Parser, Subcommand};
use luau_analyze::{CancellationToken, CheckOptions, CheckResult, Checker, checker_policy};
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

/// Fully executed check together with its display label.
#[derive(Debug)]
struct CheckExecution {
    /// Human-readable source label.
    label: String,
    /// Completed check result.
    result: CheckResult,
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

    let mut checker = build_checker(args)?;
    let cancellation_token = build_cancellation_token(args)?;
    let execution = execute_check(args, &mut checker, cancellation_token.as_ref())?;
    emit_check_output(args.json, &execution)?;
    Ok(exit_code_for_check(args, &execution.result))
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

/// Reads all bytes from stdin and decodes UTF-8 text.
fn read_stdin() -> Result<String, String> {
    let mut buffer = Vec::new();
    io::stdin()
        .read_to_end(&mut buffer)
        .map_err(|error| format!("failed to read stdin: {error}"))?;
    String::from_utf8(buffer).map_err(|error| format!("stdin is not valid UTF-8: {error}"))
}

/// Creates a checker and loads any requested definitions.
fn build_checker(args: &CheckArgs) -> Result<Checker, String> {
    let mut checker = Checker::new().map_err(|error| error.to_string())?;
    load_requested_definitions(&mut checker, args)?;
    Ok(checker)
}

/// Loads all definition files requested on the command line.
fn load_requested_definitions(checker: &mut Checker, args: &CheckArgs) -> Result<(), String> {
    if args.default_definitions {
        checker
            .add_definitions_path(Path::new(DEFAULT_DEFINITIONS))
            .map_err(|error| error.to_string())?;
    }
    for definitions in &args.definitions {
        checker
            .add_definitions_path(definitions)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Builds a pre-cancelled token when requested by the user.
fn build_cancellation_token(args: &CheckArgs) -> Result<Option<CancellationToken>, String> {
    if !args.cancel_immediately {
        return Ok(None);
    }

    let token = CancellationToken::new().map_err(|error| error.to_string())?;
    token.cancel();
    Ok(Some(token))
}

/// Executes the selected input target.
fn execute_check(
    args: &CheckArgs,
    checker: &mut Checker,
    cancellation_token: Option<&CancellationToken>,
) -> Result<CheckExecution, String> {
    match &args.file {
        Some(path) => run_path_check(checker, path, args, cancellation_token),
        None => run_stdin_check(checker, args, cancellation_token),
    }
}

/// Executes a check against a file path.
fn run_path_check(
    checker: &mut Checker,
    path: &Path,
    args: &CheckArgs,
    cancellation_token: Option<&CancellationToken>,
) -> Result<CheckExecution, String> {
    let label = path.display().to_string();
    let result = checker
        .check_path_with_options(path, build_check_options(args, None, cancellation_token))
        .map_err(|error| error.to_string())?;
    Ok(CheckExecution { label, result })
}

/// Executes a check against UTF-8 source read from stdin.
fn run_stdin_check(
    checker: &mut Checker,
    args: &CheckArgs,
    cancellation_token: Option<&CancellationToken>,
) -> Result<CheckExecution, String> {
    let source = read_stdin()?;
    let label = "<stdin>".to_owned();
    let result = checker
        .check_with_options(
            &source,
            build_check_options(args, Some(label.as_str()), cancellation_token),
        )
        .map_err(|error| error.to_string())?;
    Ok(CheckExecution { label, result })
}

/// Builds per-check options from CLI flags.
fn build_check_options<'a>(
    args: &CheckArgs,
    module_name: Option<&'a str>,
    cancellation_token: Option<&'a CancellationToken>,
) -> CheckOptions<'a> {
    CheckOptions {
        timeout: args.timeout_ms.map(Duration::from_millis),
        module_name,
        cancellation_token,
        ..CheckOptions::default()
    }
}

/// Writes check results in the selected output format.
fn emit_check_output(as_json: bool, execution: &CheckExecution) -> Result<(), String> {
    if as_json {
        let policy = checker_policy();
        let output = JsonCheckOutput {
            target: execution.label.clone(),
            ok: execution.result.is_ok(),
            solver: policy.solver,
            strict_mode: policy.strict_mode,
            timed_out: execution.result.timed_out,
            cancelled: execution.result.cancelled,
            diagnostics: diagnostics_to_json(&execution.result),
        };
        print_json(&output)
    } else {
        print_diagnostics(&execution.label, &execution.result);
        Ok(())
    }
}

/// Computes the CLI exit code from a completed check result.
fn exit_code_for_check(args: &CheckArgs, result: &CheckResult) -> ExitCode {
    if result.has_errors() || (args.fail_on_warnings && result.has_warnings()) {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
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
            severity: diagnostic.severity.as_str(),
            message: diagnostic.message.clone(),
        })
        .collect()
}

/// Prints diagnostics in a simple `file:line:col` format.
fn print_diagnostics(label: &str, result: &CheckResult) {
    for diagnostic in &result.diagnostics {
        println!(
            "{label}:{}:{}: {severity}: {}",
            diagnostic.line + 1,
            diagnostic.col + 1,
            diagnostic.message,
            severity = diagnostic.severity.as_str(),
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
