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

use std::{
    cmp::Ordering, error::Error as StdError, fmt, fs, marker::PhantomData, path::Path, ptr, slice,
    sync::Arc, time::Duration,
};

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

/// One parameter extracted from a direct functional entrypoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntrypointParam {
    /// Parameter name in source order.
    pub name: String,
    /// Type annotation text as written.
    pub annotation: String,
    /// Whether the parameter is syntactically optional.
    pub optional: bool,
}

/// Parsed schema for a direct `return function(...) ... end` chunk.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EntrypointSchema {
    /// Ordered parameter list for the returned function literal.
    pub params: Vec<EntrypointParam>,
}

impl CheckResult {
    /// Returns `true` when the result contains no errors.
    pub fn is_ok(&self) -> bool {
        !self.has_errors()
    }

    /// Returns `true` when the result contains one or more errors.
    pub fn has_errors(&self) -> bool {
        self.has_severity(Severity::Error)
    }

    /// Returns `true` when the result contains one or more warnings.
    pub fn has_warnings(&self) -> bool {
        self.has_severity(Severity::Warning)
    }

    /// Returns all error diagnostics.
    pub fn errors(&self) -> Vec<&Diagnostic> {
        self.diagnostics_with_severity(Severity::Error)
    }

    /// Returns all warning diagnostics.
    pub fn warnings(&self) -> Vec<&Diagnostic> {
        self.diagnostics_with_severity(Severity::Warning)
    }

    /// Returns all diagnostics matching the requested severity.
    fn diagnostics_with_severity(&self, severity: Severity) -> Vec<&Diagnostic> {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == severity)
            .collect()
    }

    /// Returns whether any diagnostic matches the requested severity.
    fn has_severity(&self, severity: Severity) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == severity)
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
    /// The private native checker library could not be loaded.
    NativeLibrary(String),
    /// Checker creation failed in the native layer.
    CreateCheckerFailed,
    /// Cancellation token creation failed in the native layer.
    CreateCancellationTokenFailed,
    /// Definitions failed to parse or type-check.
    Definitions(String),
    /// Entrypoint schema extraction failed.
    EntrypointSchema(String),
    /// Reading a UTF-8 text file failed before checking or loading definitions.
    ReadFile {
        /// Logical input category such as `"source"` or `"definitions"`.
        kind: &'static str,
        /// Display label for the file path.
        path: String,
        /// Human-readable I/O error message.
        message: String,
    },
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
            Self::NativeLibrary(message) => formatter.write_str(message),
            Self::CreateCheckerFailed => formatter.write_str("failed to create Luau checker"),
            Self::CreateCancellationTokenFailed => {
                formatter.write_str("failed to create Luau cancellation token")
            }
            Self::Definitions(message) => {
                write!(formatter, "failed to load Luau definitions: {message}")
            }
            Self::EntrypointSchema(message) => {
                write!(
                    formatter,
                    "failed to extract Luau entrypoint schema: {message}"
                )
            }
            Self::ReadFile {
                kind,
                path,
                message,
            } => {
                write!(formatter, "failed to read {kind} `{path}`: {message}")
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
    /// Loaded native checker entrypoints.
    api: &'static ffi::Api,
    /// Raw C cancellation token handle.
    raw: ffi::TokenHandle,
}

// The underlying C cancellation token uses atomic state and is thread-safe for signal/reset.
unsafe impl Send for CancellationTokenInner {}
// The underlying C cancellation token uses atomic state and is thread-safe for signal/reset.
unsafe impl Sync for CancellationTokenInner {}

impl Drop for CancellationTokenInner {
    fn drop(&mut self) {
        // SAFETY: `raw` originates from `luau_cancellation_token_new` and is valid until drop.
        unsafe { (self.api.luau_cancellation_token_free)(self.raw) };
    }
}

impl CancellationToken {
    /// Creates a new cancellation token.
    pub fn new() -> Result<Self, Error> {
        let api = native_api()?;
        // SAFETY: Calling into shim constructor. Null indicates failure.
        let raw = unsafe { (api.luau_cancellation_token_new)() };
        if raw.is_null() {
            return Err(Error::CreateCancellationTokenFailed);
        }
        Ok(Self {
            inner: Arc::new(CancellationTokenInner { api, raw }),
        })
    }

    /// Requests cancellation on this token.
    pub fn cancel(&self) {
        // SAFETY: `raw` is valid while `inner` is alive.
        unsafe { (self.inner.api.luau_cancellation_token_cancel)(self.inner.raw) };
    }

    /// Clears cancellation state on this token.
    pub fn reset(&self) {
        // SAFETY: `raw` is valid while `inner` is alive.
        unsafe { (self.inner.api.luau_cancellation_token_reset)(self.inner.raw) };
    }

    /// Returns the raw C token handle.
    fn raw(&self) -> ffi::TokenHandle {
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
    /// Loaded native checker entrypoints.
    api: &'static ffi::Api,
    /// Opaque handle to the native checker instance.
    inner: ffi::CheckerHandle,
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
        let api = native_api()?;
        // SAFETY: Calling into shim constructor. Null indicates failure.
        let inner = unsafe { (api.luau_checker_new)() };
        if inner.is_null() {
            return Err(Error::CreateCheckerFailed);
        }
        Ok(Self {
            api,
            inner,
            options,
        })
    }

    /// Returns immutable access to default checker options.
    pub fn options(&self) -> &CheckerOptions {
        &self.options
    }

    /// Loads Luau definition source using default module label.
    pub fn add_definitions(&mut self, defs: &str) -> Result<(), Error> {
        add_definitions_raw(
            self.api,
            self.inner,
            defs,
            &self.options.default_definitions_module_name,
        )
    }

    /// Loads Luau definitions from a UTF-8 text file using the path as module label.
    pub fn add_definitions_path(&mut self, path: &Path) -> Result<(), Error> {
        let path_label = path_label(path);
        let defs = read_utf8_path(path, "definitions")?;

        match add_definitions_raw(self.api, self.inner, &defs, &path_label) {
            Err(Error::Definitions(message)) => {
                Err(Error::Definitions(format!("{path_label}: {message}")))
            }
            other => other,
        }
    }

    /// Loads Luau definition source with an explicit module label.
    pub fn add_definitions_with_name(
        &mut self,
        defs: &str,
        module_name: &str,
    ) -> Result<(), Error> {
        add_definitions_raw(self.api, self.inner, defs, module_name)
    }

    /// Type-checks a Luau source module with default options.
    pub fn check(&mut self, source: &str) -> Result<CheckResult, Error> {
        self.check_with_options(source, CheckOptions::default())
    }

    /// Type-checks a Luau source file with default options and the path as module label.
    pub fn check_path(&mut self, path: &Path) -> Result<CheckResult, Error> {
        self.check_path_with_options(path, CheckOptions::default())
    }

    /// Type-checks a Luau source file with explicit per-call options.
    pub fn check_path_with_options(
        &mut self,
        path: &Path,
        options: CheckOptions<'_>,
    ) -> Result<CheckResult, Error> {
        let source = read_utf8_path(path, "source")?;
        let path_label = path_label(path);

        self.check_with_options(
            &source,
            CheckOptions {
                timeout: options.timeout,
                module_name: options.module_name.or(Some(path_label.as_str())),
                cancellation_token: options.cancellation_token,
            },
        )
    }

    /// Type-checks a Luau source module with explicit per-call options.
    pub fn check_with_options(
        &mut self,
        source: &str,
        options: CheckOptions<'_>,
    ) -> Result<CheckResult, Error> {
        let source = FfiStr::new(source, "source")?;

        let module_name = options
            .module_name
            .unwrap_or(self.options.default_module_name.as_str());
        let module_name = FfiStr::new(module_name, "module name")?;

        let timeout = options.timeout.or(self.options.default_timeout);
        let raw_options = ffi::LuauCheckOptions {
            module_name: module_name.ptr(),
            module_name_len: module_name.len(),
            has_timeout: u32::from(timeout.is_some()),
            timeout_seconds: timeout.map_or(0.0, |duration| duration.as_secs_f64()),
            cancellation_token: options
                .cancellation_token
                .map_or(ffi::TokenHandle::null(), CancellationToken::raw),
        };

        // SAFETY: Input pointers and checker handle are valid for call duration.
        let raw = unsafe {
            (self.api.luau_checker_check)(self.inner, source.ptr(), source.len(), &raw_options)
        };
        let raw = RawCheckResultGuard::new(self.api, raw);

        let mut diagnostics = collect_diagnostics(raw.as_ref());

        diagnostics.sort_by(diagnostic_sort_key);
        Ok(CheckResult {
            diagnostics,
            timed_out: raw.as_ref().timed_out != 0,
            cancelled: raw.as_ref().cancelled != 0,
        })
    }
}

/// Extracts parameter names, annotation text, and optionality from a direct
/// `return function(...) ... end` chunk.
pub fn extract_entrypoint_schema(source: &str) -> Result<EntrypointSchema, Error> {
    let source = FfiStr::new(source, "source")?;
    let api = native_api()?;

    // SAFETY: Input pointer is valid for the call duration.
    let raw = unsafe { (api.luau_extract_entrypoint_schema)(source.ptr(), source.len()) };
    let raw = RawEntrypointSchemaGuard::new(api, raw);

    if raw.as_ref().error_len != 0 {
        return Err(Error::EntrypointSchema(string_from_raw(
            raw.as_ref().error,
            raw.as_ref().error_len,
        )));
    }

    Ok(EntrypointSchema {
        params: collect_entrypoint_params(raw.as_ref()),
    })
}

impl Drop for Checker {
    fn drop(&mut self) {
        // SAFETY: `self.inner` originates from `luau_checker_new` and is valid until drop.
        unsafe { (self.api.luau_checker_free)(self.inner) };
    }
}

/// Loads Luau definition source through the native checker with a chosen module label.
fn add_definitions_raw(
    api: &'static ffi::Api,
    checker: ffi::CheckerHandle,
    defs: &str,
    module_name: &str,
) -> Result<(), Error> {
    let defs = FfiStr::new(defs, "definitions")?;
    let module_name = FfiStr::new(module_name, "definition module name")?;

    // SAFETY: Pointers are valid for call duration and checker handle is live.
    let raw = RawStringGuard::new(api, unsafe {
        (api.luau_checker_add_definitions)(
            checker,
            defs.ptr(),
            defs.len(),
            module_name.ptr(),
            module_name.len(),
        )
    });

    match raw.message() {
        Some(message) => Err(Error::Definitions(message)),
        None => Ok(()),
    }
}

/// Reads a UTF-8 text file used as checker input.
fn read_utf8_path(path: &Path, kind: &'static str) -> Result<String, Error> {
    let path_label = path_label(path);
    fs::read_to_string(path).map_err(|error| Error::ReadFile {
        kind,
        path: path_label,
        message: error.to_string(),
    })
}

/// Formats a path for diagnostics and module labels.
fn path_label(path: &Path) -> String {
    path.display().to_string()
}

/// Borrowed UTF-8 input prepared for a C ABI call.
#[derive(Clone, Copy)]
struct FfiStr<'a> {
    /// Pointer to the UTF-8 bytes, or null for empty strings.
    ptr: *const u8,
    /// Length of the UTF-8 payload in bytes.
    len: u32,
    /// Ties the raw pointer to the borrowed Rust string lifetime.
    _marker: PhantomData<&'a str>,
}

impl<'a> FfiStr<'a> {
    /// Converts a Rust string to a pointer-length pair accepted by the C ABI.
    fn new(value: &'a str, kind: &'static str) -> Result<Self, Error> {
        let len = u32::try_from(value.len()).map_err(|_| Error::InputTooLarge {
            kind,
            len: value.len(),
        })?;

        Ok(Self {
            ptr: if len == 0 {
                ptr::null()
            } else {
                value.as_ptr()
            },
            len,
            _marker: PhantomData,
        })
    }

    /// Returns the UTF-8 pointer for the C ABI.
    fn ptr(self) -> *const u8 {
        self.ptr
    }

    /// Returns the UTF-8 byte length for the C ABI.
    fn len(self) -> u32 {
        self.len
    }
}

/// RAII guard that releases a raw check result on scope exit.
struct RawCheckResultGuard {
    /// Loaded native checker entrypoints.
    api: &'static ffi::Api,
    /// Raw check result allocated by the shim.
    raw: ffi::LuauCheckResult,
}

impl RawCheckResultGuard {
    /// Creates a guard for a raw check result.
    fn new(api: &'static ffi::Api, raw: ffi::LuauCheckResult) -> Self {
        Self { api, raw }
    }

    /// Returns a shared reference to the raw check result.
    fn as_ref(&self) -> &ffi::LuauCheckResult {
        &self.raw
    }
}

impl Drop for RawCheckResultGuard {
    fn drop(&mut self) {
        // SAFETY: `raw` came from shim and must be released exactly once.
        unsafe { (self.api.luau_check_result_free)(self.raw) };
    }
}

/// RAII guard that releases a raw string result on scope exit.
struct RawStringGuard {
    /// Loaded native checker entrypoints.
    api: &'static ffi::Api,
    /// Raw string result allocated by the shim.
    raw: ffi::LuauString,
}

impl RawStringGuard {
    /// Creates a guard for a raw string result.
    fn new(api: &'static ffi::Api, raw: ffi::LuauString) -> Self {
        Self { api, raw }
    }

    /// Reads the string payload when the shim returned one.
    fn message(&self) -> Option<String> {
        if self.raw.len == 0 {
            None
        } else {
            Some(string_from_raw(self.raw.data, self.raw.len))
        }
    }
}

impl Drop for RawStringGuard {
    fn drop(&mut self) {
        // SAFETY: `raw` came from shim and must be released exactly once.
        unsafe { (self.api.luau_string_free)(self.raw) };
    }
}

/// RAII guard that releases a raw entrypoint schema result on scope exit.
struct RawEntrypointSchemaGuard {
    /// Loaded native checker entrypoints.
    api: &'static ffi::Api,
    /// Raw entrypoint schema result allocated by the shim.
    raw: ffi::LuauEntrypointSchemaResult,
}

impl RawEntrypointSchemaGuard {
    /// Creates a guard for a raw entrypoint schema result.
    fn new(api: &'static ffi::Api, raw: ffi::LuauEntrypointSchemaResult) -> Self {
        Self { api, raw }
    }

    /// Returns a shared reference to the raw result.
    fn as_ref(&self) -> &ffi::LuauEntrypointSchemaResult {
        &self.raw
    }
}

impl Drop for RawEntrypointSchemaGuard {
    fn drop(&mut self) {
        // SAFETY: `raw` came from shim and must be released exactly once.
        unsafe { (self.api.luau_entrypoint_schema_result_free)(self.raw) };
    }
}

/// Returns the loaded native checker entrypoints.
fn native_api() -> Result<&'static ffi::Api, Error> {
    ffi::api().map_err(Error::NativeLibrary)
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

impl Severity {
    /// Converts the shim severity code into the public enum.
    fn from_ffi(code: u32) -> Self {
        match code {
            0 => Self::Error,
            _ => Self::Warning,
        }
    }
}

/// Converts diagnostic rows owned by the shim into Rust values.
fn collect_diagnostics(raw: &ffi::LuauCheckResult) -> Vec<Diagnostic> {
    // SAFETY: `raw.diagnostics` points to `diagnostic_count` entries owned by `raw`.
    unsafe { raw_slice(raw.diagnostics, raw.diagnostic_count) }
        .iter()
        .map(|diagnostic| Diagnostic {
            line: diagnostic.line,
            col: diagnostic.col,
            end_line: diagnostic.end_line,
            end_col: diagnostic.end_col,
            severity: Severity::from_ffi(diagnostic.severity),
            message: string_from_raw(diagnostic.message, diagnostic.message_len),
        })
        .collect()
}

/// Converts entrypoint parameter rows owned by the shim into Rust values.
fn collect_entrypoint_params(raw: &ffi::LuauEntrypointSchemaResult) -> Vec<EntrypointParam> {
    // SAFETY: `raw.params` points to `param_count` entries owned by `raw`.
    unsafe { raw_slice(raw.params, raw.param_count) }
        .iter()
        .map(|param| EntrypointParam {
            name: string_from_raw(param.name, param.name_len),
            annotation: string_from_raw(param.annotation, param.annotation_len),
            optional: param.optional != 0,
        })
        .collect()
}

/// Forms a borrowed slice from a non-owning C pointer and element count.
unsafe fn raw_slice<'a, T>(ptr: *const T, len: u32) -> &'a [T] {
    if len == 0 {
        &[]
    } else {
        debug_assert!(!ptr.is_null(), "non-empty shim slice must not be null");
        // SAFETY: The caller guarantees `ptr` is valid for `len` elements.
        unsafe { slice::from_raw_parts(ptr, len as usize) }
    }
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
    use std::{
        env, fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        CheckResult, Checker, CheckerOptions, Diagnostic, Error, Severity, checker_policy,
        extract_entrypoint_schema,
    };

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

    /// Verifies schema extraction reads direct function parameters in order.
    #[test]
    fn extract_entrypoint_schema_reads_params() {
        let schema = extract_entrypoint_schema(
            r#"
return function(target: Node, count: number?, payload: JsonValue)
    return nil
end
"#,
        )
        .expect("schema");
        assert_eq!(3, schema.params.len());
        assert_eq!("target", schema.params[0].name);
        assert_eq!("Node", schema.params[0].annotation);
        assert!(!schema.params[0].optional);
        assert_eq!("count", schema.params[1].name);
        assert_eq!("number?", schema.params[1].annotation);
        assert!(schema.params[1].optional);
        assert_eq!("payload", schema.params[2].name);
        assert_eq!("JsonValue", schema.params[2].annotation);
        assert!(!schema.params[2].optional);
    }

    /// Verifies schema extraction rejects indirect entrypoints.
    #[test]
    fn extract_entrypoint_schema_rejects_indirect_return() {
        let error = extract_entrypoint_schema(
            r#"
local main = function(target: Node)
    return nil
end
return main
"#,
        )
        .expect_err("schema should fail");
        assert!(
            error
                .to_string()
                .contains("script must use a direct `return function(...) ... end` entrypoint"),
            "{error}"
        );
    }

    /// Verifies path-based source checks surface readable file errors.
    #[test]
    fn check_path_reports_read_error() {
        let mut checker = Checker::new().expect("checker creation should succeed");
        let missing = temp_path("missing_source");

        let error = checker
            .check_path(&missing)
            .expect_err("missing file should fail");
        match error {
            Error::ReadFile {
                kind,
                path,
                message,
            } => {
                assert_eq!("source", kind);
                assert_eq!(missing.display().to_string(), path);
                assert!(
                    !message.is_empty(),
                    "read error message should not be empty"
                );
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    /// Verifies path-based definitions loading reads UTF-8 files and preserves labels.
    #[test]
    fn add_definitions_path_loads_file_contents() {
        let mut checker = Checker::new().expect("checker creation should succeed");
        let path = temp_path("definitions");
        fs::write(&path, "declare function file_defined(): string\n")
            .expect("definitions file should be written");

        checker
            .add_definitions_path(&path)
            .expect("definitions path should load");
        let result = checker
            .check(
                r#"
            --!strict
            local value: string = file_defined()
            "#,
            )
            .expect("source should check");

        fs::remove_file(&path).expect("temp file should be removed");
        assert!(result.is_ok(), "path-loaded definitions should stay active");
    }

    /// Creates a unique temp file path for filesystem tests.
    fn temp_path(stem: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        env::temp_dir().join(format!("luau-analyze-{stem}-{unique}.luau"))
    }
}
