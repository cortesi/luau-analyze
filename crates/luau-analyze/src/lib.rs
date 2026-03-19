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

use std::{cmp::Ordering, error::Error as StdError, fmt, ptr, slice, sync::Arc, time::Duration};

/// Default module label for source checks.
const DEFAULT_CHECK_MODULE_NAME: &str = "main";
/// Default module label for definition loading.
const DEFAULT_DEFINITIONS_MODULE_NAME: &str = "@definitions";

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

/// Result of a single checker run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckResult {
    /// Collected diagnostics sorted by location and severity.
    pub diagnostics: Vec<Diagnostic>,
    /// Whether the check hit one or more time limits.
    pub timed_out: bool,
    /// Whether cancellation was requested during checking.
    pub cancelled: bool,
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

    /// Returns `true` when the check exceeded its configured time limit.
    pub fn timed_out(&self) -> bool {
        self.timed_out
    }

    /// Returns `true` when cancellation was requested for this check.
    pub fn cancelled(&self) -> bool {
        self.cancelled
    }
}

/// Stable checker policy values exposed by this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckerPolicy {
    /// Whether strict mode is always enforced.
    pub strict_mode: bool,
    /// Active solver policy string.
    pub solver: &'static str,
    /// Whether batch queue support is exposed by this crate.
    pub exposes_batch_queue: bool,
}

/// Returns the current fixed checker policy.
pub const fn checker_policy() -> CheckerPolicy {
    CheckerPolicy {
        strict_mode: true,
        solver: "new",
        exposes_batch_queue: false,
    }
}

/// Errors returned by checker construction and definition loading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Checker creation failed in the native layer.
    CreateCheckerFailed,
    /// Cancellation token creation failed in the native layer.
    CreateCancellationTokenFailed,
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
            Self::CreateCancellationTokenFailed => {
                formatter.write_str("failed to create Luau cancellation token")
            }
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

/// Default checker configuration used by `Checker`.
#[derive(Debug, Clone)]
pub struct CheckerOptions {
    /// Optional timeout applied to checks that do not override it.
    pub default_timeout: Option<Duration>,
    /// Default module label used for source checks.
    pub default_module_name: String,
    /// Default module label used for definition loading.
    pub default_definitions_module_name: String,
}

impl Default for CheckerOptions {
    fn default() -> Self {
        Self {
            default_timeout: None,
            default_module_name: DEFAULT_CHECK_MODULE_NAME.to_owned(),
            default_definitions_module_name: DEFAULT_DEFINITIONS_MODULE_NAME.to_owned(),
        }
    }
}

/// Per-call options for `Checker::check_with_options`.
#[derive(Debug, Clone, Copy, Default)]
pub struct CheckOptions<'a> {
    /// Optional timeout override for this call.
    pub timeout: Option<Duration>,
    /// Optional module label override for this call.
    pub module_name: Option<&'a str>,
    /// Optional cancellation token for this call.
    pub cancellation_token: Option<&'a CancellationToken>,
}

/// A reusable cancellation token that can be signaled from another thread.
///
/// `CancellationToken` is `Send` and `Sync` because the underlying Luau implementation
/// uses atomic operations to manage its signaled state safely across thread boundaries.
#[derive(Clone, Debug)]
pub struct CancellationToken {
    /// Shared token internals.
    inner: Arc<CancellationTokenInner>,
}

/// Shared cancellation token internals.
#[derive(Debug)]
struct CancellationTokenInner {
    /// Raw C cancellation token handle.
    raw: *mut ffi::LuauCancellationToken,
}

// The underlying C cancellation token uses atomic state and is thread-safe for signal/reset.
unsafe impl Send for CancellationTokenInner {}
// The underlying C cancellation token uses atomic state and is thread-safe for signal/reset.
unsafe impl Sync for CancellationTokenInner {}

impl Drop for CancellationTokenInner {
    fn drop(&mut self) {
        // SAFETY: `raw` originates from `luau_cancellation_token_new` and is valid until drop.
        unsafe { ffi::luau_cancellation_token_free(self.raw) };
    }
}

impl CancellationToken {
    /// Creates a new cancellation token.
    pub fn new() -> Result<Self, Error> {
        // SAFETY: Calling into shim constructor. Null indicates failure.
        let raw = unsafe { ffi::luau_cancellation_token_new() };
        if raw.is_null() {
            return Err(Error::CreateCancellationTokenFailed);
        }
        Ok(Self {
            inner: Arc::new(CancellationTokenInner { raw }),
        })
    }

    /// Requests cancellation on this token.
    pub fn cancel(&self) {
        // SAFETY: `raw` is valid while `inner` is alive.
        unsafe { ffi::luau_cancellation_token_cancel(self.inner.raw) };
    }

    /// Clears cancellation state on this token.
    pub fn reset(&self) {
        // SAFETY: `raw` is valid while `inner` is alive.
        unsafe { ffi::luau_cancellation_token_reset(self.inner.raw) };
    }

    /// Returns the raw C token pointer.
    fn raw(&self) -> *mut ffi::LuauCancellationToken {
        self.inner.raw
    }
}

/// Reusable checker instance with persistent global definitions.
///
/// `Checker` is `Send` but not `Sync`. The underlying Luau Analysis structures
/// are safely movable between threads, but all operations that mutate or read
/// from the checker require exclusive `&mut self` access, meaning it cannot
/// be concurrently accessed from multiple threads.
pub struct Checker {
    /// Opaque pointer to the native checker instance.
    inner: *mut ffi::LuauChecker,
    /// Default checker behavior options.
    options: CheckerOptions,
}

// The underlying checker is single-threaded (`&mut self` methods), but ownership can move.
unsafe impl Send for Checker {}

impl Checker {
    /// Creates a checker with default options.
    pub fn new() -> Result<Self, Error> {
        Self::with_options(CheckerOptions::default())
    }

    /// Creates a checker with explicit defaults.
    pub fn with_options(options: CheckerOptions) -> Result<Self, Error> {
        // SAFETY: Calling into shim constructor. Null indicates failure.
        let inner = unsafe { ffi::luau_checker_new() };
        if inner.is_null() {
            return Err(Error::CreateCheckerFailed);
        }

        Ok(Self { inner, options })
    }

    /// Returns immutable access to default checker options.
    pub fn options(&self) -> &CheckerOptions {
        &self.options
    }

    /// Loads Luau definition source using default module label.
    pub fn add_definitions(&mut self, defs: &str) -> Result<(), Error> {
        let module_name = self.options.default_definitions_module_name.clone();
        self.add_definitions_with_name(defs, &module_name)
    }

    /// Loads Luau definition source with an explicit module label.
    pub fn add_definitions_with_name(
        &mut self,
        defs: &str,
        module_name: &str,
    ) -> Result<(), Error> {
        let (defs_ptr, defs_len) = ffi_str(defs, "definitions")?;
        let (module_name_ptr, module_name_len) =
            ffi_optional_str(module_name, "definition module name")?;

        // SAFETY: Pointers are valid for call duration and checker handle is live.
        let raw = unsafe {
            ffi::luau_checker_add_definitions(
                self.inner,
                defs_ptr,
                defs_len,
                module_name_ptr,
                module_name_len,
            )
        };

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

    /// Type-checks a Luau source module with default options.
    pub fn check(&mut self, source: &str) -> Result<CheckResult, Error> {
        self.check_with_options(source, CheckOptions::default())
    }

    /// Type-checks a Luau source module with explicit per-call options.
    pub fn check_with_options(
        &mut self,
        source: &str,
        options: CheckOptions<'_>,
    ) -> Result<CheckResult, Error> {
        let (source_ptr, source_len) = ffi_str(source, "source")?;

        let module_name = options
            .module_name
            .unwrap_or(self.options.default_module_name.as_str());
        let (module_name_ptr, module_name_len) = ffi_optional_str(module_name, "module name")?;

        let timeout = options.timeout.or(self.options.default_timeout);
        let raw_options = ffi::LuauCheckOptions {
            module_name: module_name_ptr,
            module_name_len,
            has_timeout: u32::from(timeout.is_some()),
            timeout_seconds: timeout.map_or(0.0, |duration| duration.as_secs_f64()),
            cancellation_token: options
                .cancellation_token
                .map_or(ptr::null_mut(), CancellationToken::raw),
        };

        // SAFETY: Input pointers and checker handle are valid for call duration.
        let raw =
            unsafe { ffi::luau_checker_check(self.inner, source_ptr, source_len, &raw_options) };
        let raw = RawCheckResultGuard::new(raw);

        let mut diagnostics = if raw.as_ref().diagnostic_count == 0 {
            Vec::new()
        } else {
            // SAFETY: `raw.diagnostics` points to `diagnostic_count` entries owned by `raw`.
            let diagnostics_slice = unsafe {
                slice::from_raw_parts(
                    raw.as_ref().diagnostics,
                    raw.as_ref().diagnostic_count as usize,
                )
            };
            diagnostics_slice
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
        Ok(CheckResult {
            diagnostics,
            timed_out: raw.as_ref().timed_out != 0,
            cancelled: raw.as_ref().cancelled != 0,
        })
    }
}

impl Drop for Checker {
    fn drop(&mut self) {
        // SAFETY: `self.inner` originates from `luau_checker_new` and is valid until drop.
        unsafe { ffi::luau_checker_free(self.inner) };
    }
}

/// RAII guard that releases a raw check result on scope exit.
struct RawCheckResultGuard {
    /// Raw check result allocated by the shim.
    raw: ffi::LuauCheckResult,
}

impl RawCheckResultGuard {
    /// Creates a guard for a raw check result.
    fn new(raw: ffi::LuauCheckResult) -> Self {
        Self { raw }
    }

    /// Returns a shared reference to the raw check result.
    fn as_ref(&self) -> &ffi::LuauCheckResult {
        &self.raw
    }
}

impl Drop for RawCheckResultGuard {
    fn drop(&mut self) {
        // SAFETY: `raw` came from shim and must be released exactly once.
        unsafe { ffi::luau_check_result_free(self.raw) };
    }
}

/// Converts a required Rust string to C pointer and `u32` length.
fn ffi_str(value: &str, kind: &'static str) -> Result<(*const u8, u32), Error> {
    let len = u32::try_from(value.len()).map_err(|_| Error::InputTooLarge {
        kind,
        len: value.len(),
    })?;

    if len == 0 {
        Ok((ptr::null(), 0))
    } else {
        Ok((value.as_ptr(), len))
    }
}

/// Converts an optional-ish Rust string to C pointer and `u32` length.
fn ffi_optional_str(value: &str, kind: &'static str) -> Result<(*const u8, u32), Error> {
    ffi_str(value, kind)
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

/// Unit tests for public result helpers and policy defaults.
#[cfg(test)]
mod tests {
    use super::{CheckResult, CheckerOptions, Diagnostic, Severity, checker_policy};

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
            timed_out: false,
            cancelled: false,
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
            timed_out: false,
            cancelled: false,
        };

        assert!(!result.is_ok());
        assert_eq!(0, result.warnings().len());
        assert_eq!(1, result.errors().len());
    }

    /// Verifies policy constants match project decisions.
    #[test]
    fn policy_is_strict_new_solver_and_queue_free() {
        let policy = checker_policy();
        assert!(policy.strict_mode);
        assert_eq!("new", policy.solver);
        assert!(!policy.exposes_batch_queue);
    }

    /// Verifies checker options defaults use stable module labels.
    #[test]
    fn checker_options_defaults_are_stable() {
        let options = CheckerOptions::default();
        assert_eq!("main", options.default_module_name);
        assert_eq!("@definitions", options.default_definitions_module_name);
        assert!(options.default_timeout.is_none());
    }
}
