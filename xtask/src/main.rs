//! Project-local task runner.

use std::{
    path::PathBuf,
    process::{Command, ExitCode},
};

use clap::{Parser, Subcommand};

/// Top-level command-line arguments.
#[derive(Debug, Parser)]
#[command(about = "Project task runner")]
struct Args {
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
}

/// Parses arguments and dispatches to the selected subcommand.
fn main() -> ExitCode {
    let args = Args::parse();
    let result = match args.command {
        XtaskCommand::Tidy => tidy(),
        XtaskCommand::Test => test(),
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
