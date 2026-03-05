//! In-process Luau type checking for Rust.
//!
//! # Example
//!
//! ```no_run
//! use luau_analyze::Checker;
//!
//! let mut checker = Checker::new().expect("checker should initialize");
//! checker
//!     .add_definitions(
//!         r#"
//!         declare class TodoBuilder
//!             function content(self, content: string): TodoBuilder
//!         end
//!         declare Todo: { create: () -> TodoBuilder }
//!         "#,
//!     )
//!     .expect("definitions should load");
//!
//! let result = checker.check(
//!     r#"
//!     --!strict
//!     local _todo = Todo.create():content("review")
//!     "#,
//! );
//! assert!(result.is_ok());
//! ```

/// Low-level FFI declarations for the Luau analysis bridge.
mod ffi;

use std::{cmp::Ordering, error::Error as StdError, fmt, slice};

/// Diagnostic severity emitted by the checker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Type-check or lint error.
    Error,
    /// Lint warning.
    Warning,
}

/// A single diagnostic item from checking Luau source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Zero-based start line.
    pub line: u32,
    /// Zero-based start column.
    pub col: u32,
    /// Zero-based end line.
    pub end_line: u32,
    /// Zero-based end column.
    pub end_col: u32,
    /// Severity level.
    pub severity: Severity,
    /// Human-readable diagnostic message.
    pub message: String,
}

/// Result of a single `Checker::check` run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckResult {
    /// Collected diagnostics sorted by location and severity.
    pub diagnostics: Vec<Diagnostic>,
}

impl CheckResult {
    /// Returns `true` when the result contains no errors.
    pub fn is_ok(&self) -> bool {
        !self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }

    /// Returns all error diagnostics.
    pub fn errors(&self) -> Vec<&Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
            .collect()
    }

    /// Returns all warning diagnostics.
    pub fn warnings(&self) -> Vec<&Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Warning)
            .collect()
    }
}

/// Errors returned by checker construction and definition loading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Checker creation failed in the native layer.
    CreateCheckerFailed,
    /// Definitions failed to parse or type-check.
    Definitions(String),
    /// UTF-8 input is too large for the C ABI length type.
    InputTooLarge {
        /// Logical input category such as `"source"` or `"definitions"`.
        kind: &'static str,
        /// Original input length in bytes.
        len: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CreateCheckerFailed => formatter.write_str("failed to create Luau checker"),
            Self::Definitions(message) => {
                write!(formatter, "failed to load Luau definitions: {message}")
            }
            Self::InputTooLarge { kind, len } => {
                write!(
                    formatter,
                    "{kind} input is too large for checker FFI boundary ({len} bytes)"
                )
            }
        }
    }
}

impl StdError for Error {}

/// Reusable checker instance with persistent global definitions.
pub struct Checker {
    /// Opaque pointer to the native checker instance.
    inner: *mut ffi::LuauChecker,
}

// The underlying checker is single-threaded (`&mut self` methods), but ownership can move.
unsafe impl Send for Checker {}

impl Checker {
    /// Creates a new checker with strict mode config and Luau builtins.
    pub fn new() -> Result<Self, Error> {
        // SAFETY: Calling into shim constructor. Null indicates failure.
        let inner = unsafe { ffi::luau_checker_new() };
        if inner.is_null() {
            return Err(Error::CreateCheckerFailed);
        }
        Ok(Self { inner })
    }

    /// Loads Luau definition source that augments checker globals.
    pub fn add_definitions(&mut self, defs: &str) -> Result<(), Error> {
        let defs_len = u32::try_from(defs.len()).map_err(|_| Error::InputTooLarge {
            kind: "definitions",
            len: defs.len(),
        })?;

        // SAFETY: `self.inner` is a valid checker pointer while `self` is alive.
        let raw = unsafe { ffi::luau_checker_add_definitions(self.inner, defs.as_ptr(), defs_len) };
        let error_message = if raw.len == 0 {
            None
        } else {
            Some(string_from_raw(raw.data, raw.len))
        };
        // SAFETY: `raw` came from shim and must be released exactly once.
        unsafe { ffi::luau_string_free(raw) };

        match error_message {
            Some(message) => Err(Error::Definitions(message)),
            None => Ok(()),
        }
    }

    /// Type-checks a Luau source module and returns all diagnostics.
    pub fn check(&mut self, source: &str) -> CheckResult {
        let source_len = match u32::try_from(source.len()) {
            Ok(value) => value,
            Err(_) => {
                return CheckResult {
                    diagnostics: vec![Diagnostic {
                        line: 0,
                        col: 0,
                        end_line: 0,
                        end_col: 0,
                        severity: Severity::Error,
                        message: format!(
                            "{}",
                            Error::InputTooLarge {
                                kind: "source",
                                len: source.len(),
                            }
                        ),
                    }],
                };
            }
        };

        // SAFETY: `self.inner` is valid and `source` bytes live for call duration.
        let raw = unsafe { ffi::luau_checker_check(self.inner, source.as_ptr(), source_len) };
        let mut diagnostics = if raw.diagnostic_count == 0 {
            Vec::new()
        } else {
            // SAFETY: `raw.diagnostics` points to `diagnostic_count` entries owned by `raw`.
            let slice =
                unsafe { slice::from_raw_parts(raw.diagnostics, raw.diagnostic_count as usize) };
            slice
                .iter()
                .map(|diagnostic| Diagnostic {
                    line: diagnostic.line,
                    col: diagnostic.col,
                    end_line: diagnostic.end_line,
                    end_col: diagnostic.end_col,
                    severity: if diagnostic.severity == 0 {
                        Severity::Error
                    } else {
                        Severity::Warning
                    },
                    message: string_from_raw(diagnostic.message, diagnostic.message_len),
                })
                .collect::<Vec<_>>()
        };
        diagnostics.sort_by(diagnostic_sort_key);

        // SAFETY: `raw` came from shim and must be released exactly once.
        unsafe { ffi::luau_check_result_free(raw) };

        CheckResult { diagnostics }
    }
}

impl Drop for Checker {
    fn drop(&mut self) {
        // SAFETY: `self.inner` originates from `luau_checker_new` and is valid until drop.
        unsafe { ffi::luau_checker_free(self.inner) };
    }
}

impl Default for Checker {
    fn default() -> Self {
        Self::new().expect("checker creation should succeed")
    }
}

/// Converts raw UTF-8 bytes from C into a Rust `String`.
fn string_from_raw(ptr: *const u8, len: u32) -> String {
    if ptr.is_null() || len == 0 {
        return String::new();
    }

    // SAFETY: `ptr` points to `len` bytes provided by the shim for this call scope.
    let bytes = unsafe { slice::from_raw_parts(ptr, len as usize) };
    String::from_utf8_lossy(bytes).into_owned()
}

/// Sorts diagnostics by location, then severity, then message.
fn diagnostic_sort_key(left: &Diagnostic, right: &Diagnostic) -> Ordering {
    left.line
        .cmp(&right.line)
        .then(left.col.cmp(&right.col))
        .then(left.severity.cmp(&right.severity))
        .then(left.message.cmp(&right.message))
}

/// Unit tests for the scaffold crate.
#[cfg(test)]
mod tests {
    use super::{CheckResult, Diagnostic, Severity};

    /// Verifies `CheckResult::is_ok` is true for warning-only results.
    #[test]
    fn check_result_ok_with_warnings() {
        let result = CheckResult {
            diagnostics: vec![Diagnostic {
                line: 0,
                col: 0,
                end_line: 0,
                end_col: 1,
                severity: Severity::Warning,
                message: "unused local".to_owned(),
            }],
        };

        assert!(result.is_ok());
        assert_eq!(1, result.warnings().len());
        assert_eq!(0, result.errors().len());
    }

    /// Verifies `CheckResult::is_ok` is false when at least one error exists.
    #[test]
    fn check_result_not_ok_with_error() {
        let result = CheckResult {
            diagnostics: vec![Diagnostic {
                line: 1,
                col: 1,
                end_line: 1,
                end_col: 5,
                severity: Severity::Error,
                message: "type mismatch".to_owned(),
            }],
        };

        assert!(!result.is_ok());
        assert_eq!(0, result.warnings().len());
        assert_eq!(1, result.errors().len());
    }
}
