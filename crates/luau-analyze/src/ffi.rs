//! Raw FFI bindings for the Luau analysis shim.

use std::ffi::c_void;

/// C ABI diagnostic structure emitted by the shim.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct LuauDiagnostic {
    /// Zero-based starting line.
    pub(crate) line: u32,
    /// Zero-based starting column.
    pub(crate) col: u32,
    /// Zero-based ending line.
    pub(crate) end_line: u32,
    /// Zero-based ending column.
    pub(crate) end_col: u32,
    /// Severity code where `0` is error and `1` is warning.
    pub(crate) severity: u32,
    /// Pointer to UTF-8 bytes owned by the C side.
    pub(crate) message: *const u8,
    /// Length of `message`.
    pub(crate) message_len: u32,
}

/// C ABI result object containing diagnostic storage.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct LuauCheckResult {
    /// Internal opaque pointer owned by C.
    pub(crate) _internal: *mut c_void,
    /// Pointer to diagnostic array owned by C.
    pub(crate) diagnostics: *const LuauDiagnostic,
    /// Number of diagnostics in `diagnostics`.
    pub(crate) diagnostic_count: u32,
    /// Whether the check hit one or more time limits.
    pub(crate) timed_out: u32,
    /// Whether cancellation was requested during checking.
    pub(crate) cancelled: u32,
}

/// C ABI string result used for definition-load failures.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct LuauString {
    /// Internal opaque pointer owned by C.
    pub(crate) _internal: *mut c_void,
    /// Pointer to UTF-8 bytes owned by C.
    pub(crate) data: *const u8,
    /// Length of `data`.
    pub(crate) len: u32,
}

/// C ABI entrypoint parameter row emitted by the shim.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct LuauEntrypointParam {
    /// Pointer to UTF-8 parameter name bytes owned by the C side.
    pub(crate) name: *const u8,
    /// Length of `name`.
    pub(crate) name_len: u32,
    /// Pointer to UTF-8 annotation bytes owned by the C side.
    pub(crate) annotation: *const u8,
    /// Length of `annotation`.
    pub(crate) annotation_len: u32,
    /// Whether the parameter is syntactically optional.
    pub(crate) optional: u32,
}

/// C ABI entrypoint schema extraction result.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct LuauEntrypointSchemaResult {
    /// Internal opaque pointer owned by C.
    pub(crate) _internal: *mut c_void,
    /// Pointer to parameter rows owned by C.
    pub(crate) params: *const LuauEntrypointParam,
    /// Number of parameters in `params`.
    pub(crate) param_count: u32,
    /// Pointer to UTF-8 error message bytes owned by C.
    pub(crate) error: *const u8,
    /// Length of `error`.
    pub(crate) error_len: u32,
}

/// C ABI check options passed into a single checker invocation.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct LuauCheckOptions {
    /// Optional module name label used for diagnostics.
    pub(crate) module_name: *const u8,
    /// Length of `module_name`.
    pub(crate) module_name_len: u32,
    /// Whether timeout is set (`0` or `1`).
    pub(crate) has_timeout: u32,
    /// Timeout in seconds when `has_timeout` is non-zero.
    pub(crate) timeout_seconds: f64,
    /// Optional cancellation token pointer.
    pub(crate) cancellation_token: *mut LuauCancellationToken,
}

/// Opaque C checker handle.
pub enum LuauChecker {}
/// Opaque C cancellation token handle.
pub enum LuauCancellationToken {}

unsafe extern "C" {
    /// Creates a new checker instance.
    pub(crate) fn luau_checker_new() -> *mut LuauChecker;
    /// Frees a checker instance.
    pub(crate) fn luau_checker_free(checker: *mut LuauChecker);
    /// Creates a cancellation token.
    pub(crate) fn luau_cancellation_token_new() -> *mut LuauCancellationToken;
    /// Frees a cancellation token.
    pub(crate) fn luau_cancellation_token_free(token: *mut LuauCancellationToken);
    /// Marks a cancellation token as cancelled.
    pub(crate) fn luau_cancellation_token_cancel(token: *mut LuauCancellationToken);
    /// Clears cancellation state on a token.
    pub(crate) fn luau_cancellation_token_reset(token: *mut LuauCancellationToken);

    /// Loads definition source into the checker.
    pub(crate) fn luau_checker_add_definitions(
        checker: *mut LuauChecker,
        defs: *const u8,
        defs_len: u32,
        module_name: *const u8,
        module_name_len: u32,
    ) -> LuauString;
    /// Type-checks a source module.
    pub(crate) fn luau_checker_check(
        checker: *mut LuauChecker,
        source: *const u8,
        source_len: u32,
        options: *const LuauCheckOptions,
    ) -> LuauCheckResult;
    /// Extracts a direct functional entrypoint schema from source.
    pub(crate) fn luau_extract_entrypoint_schema(
        source: *const u8,
        source_len: u32,
    ) -> LuauEntrypointSchemaResult;
    /// Frees a check result.
    pub(crate) fn luau_check_result_free(result: LuauCheckResult);
    /// Frees an entrypoint schema result.
    pub(crate) fn luau_entrypoint_schema_result_free(result: LuauEntrypointSchemaResult);
    /// Frees a C ABI string.
    pub(crate) fn luau_string_free(value: LuauString);
}
