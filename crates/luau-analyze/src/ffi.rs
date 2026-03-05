//! Raw FFI bindings for the Luau analysis shim.

use std::ffi::c_void;

/// C ABI diagnostic structure emitted by the shim.
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
#[repr(C)]
pub struct LuauCheckResult {
    /// Internal opaque pointer owned by C.
    pub(crate) _internal: *mut c_void,
    /// Pointer to diagnostic array owned by C.
    pub(crate) diagnostics: *const LuauDiagnostic,
    /// Number of diagnostics in `diagnostics`.
    pub(crate) diagnostic_count: u32,
}

/// C ABI string result used for definition-load failures.
#[repr(C)]
pub struct LuauString {
    /// Internal opaque pointer owned by C.
    pub(crate) _internal: *mut c_void,
    /// Pointer to UTF-8 bytes owned by C.
    pub(crate) data: *const u8,
    /// Length of `data`.
    pub(crate) len: u32,
}

/// Opaque C checker handle.
pub enum LuauChecker {}

unsafe extern "C" {
    /// Creates a new checker instance.
    pub(crate) fn luau_checker_new() -> *mut LuauChecker;
    /// Frees a checker instance.
    pub(crate) fn luau_checker_free(checker: *mut LuauChecker);
    /// Loads definition source into the checker.
    pub(crate) fn luau_checker_add_definitions(
        checker: *mut LuauChecker,
        defs: *const u8,
        defs_len: u32,
    ) -> LuauString;
    /// Type-checks a source module.
    pub(crate) fn luau_checker_check(
        checker: *mut LuauChecker,
        source: *const u8,
        source_len: u32,
    ) -> LuauCheckResult;
    /// Frees a check result.
    pub(crate) fn luau_check_result_free(result: LuauCheckResult);
    /// Frees a C ABI string.
    pub(crate) fn luau_string_free(value: LuauString);
}
