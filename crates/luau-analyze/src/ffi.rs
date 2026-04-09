//! Raw FFI bindings and native loader for the Luau analysis shim.

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    env,
    ffi::c_void,
    fs,
    path::{Path, PathBuf},
    process,
    ptr::null_mut,
    sync::OnceLock,
};

use libloading::Library;

/// File name for the private native checker library.
const NATIVE_LIBRARY_FILE_NAME: &str = env!("LUAU_ANALYZE_NATIVE_LIB_FILE_NAME");
/// Embedded native checker library bytes.
static NATIVE_LIBRARY_BYTES: &[u8] = include_bytes!(env!("LUAU_ANALYZE_NATIVE_LIB_PATH"));

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
    /// Optional cancellation token handle.
    pub(crate) cancellation_token: TokenHandle,
}

/// Opaque checker handle returned by the native shim.
#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub struct CheckerHandle(*mut c_void);

impl CheckerHandle {
    /// Returns whether the handle is null.
    pub(crate) fn is_null(self) -> bool {
        self.0.is_null()
    }
}

/// Opaque cancellation token handle returned by the native shim.
#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub struct TokenHandle(*mut c_void);

impl TokenHandle {
    /// Returns a null token handle.
    pub(crate) const fn null() -> Self {
        Self(null_mut())
    }

    /// Returns whether the handle is null.
    pub(crate) fn is_null(self) -> bool {
        self.0.is_null()
    }
}

/// Loaded native shim entrypoints.
#[derive(Debug)]
pub struct Api {
    /// Creates a new checker instance.
    pub(crate) luau_checker_new: unsafe extern "C" fn() -> CheckerHandle,
    /// Frees a checker instance.
    pub(crate) luau_checker_free: unsafe extern "C" fn(CheckerHandle),
    /// Creates a cancellation token.
    pub(crate) luau_cancellation_token_new: unsafe extern "C" fn() -> TokenHandle,
    /// Frees a cancellation token.
    pub(crate) luau_cancellation_token_free: unsafe extern "C" fn(TokenHandle),
    /// Marks a cancellation token as cancelled.
    pub(crate) luau_cancellation_token_cancel: unsafe extern "C" fn(TokenHandle),
    /// Clears cancellation state on a token.
    pub(crate) luau_cancellation_token_reset: unsafe extern "C" fn(TokenHandle),
    /// Loads definition source into the checker.
    pub(crate) luau_checker_add_definitions:
        unsafe extern "C" fn(CheckerHandle, *const u8, u32, *const u8, u32) -> LuauString,
    /// Type-checks a source module.
    pub(crate) luau_checker_check: unsafe extern "C" fn(
        CheckerHandle,
        *const u8,
        u32,
        *const LuauCheckOptions,
    ) -> LuauCheckResult,
    /// Extracts a direct functional entrypoint schema from source.
    pub(crate) luau_extract_entrypoint_schema:
        unsafe extern "C" fn(*const u8, u32) -> LuauEntrypointSchemaResult,
    /// Frees a check result.
    pub(crate) luau_check_result_free: unsafe extern "C" fn(LuauCheckResult),
    /// Frees an entrypoint schema result.
    pub(crate) luau_entrypoint_schema_result_free: unsafe extern "C" fn(LuauEntrypointSchemaResult),
    /// Frees a C ABI string.
    pub(crate) luau_string_free: unsafe extern "C" fn(LuauString),
}

/// Global native entrypoint cache.
static API: OnceLock<Result<Api, String>> = OnceLock::new();
/// Global materialized native library path cache.
static MATERIALIZED_LIBRARY_PATH: OnceLock<Result<PathBuf, String>> = OnceLock::new();

/// Returns the loaded native checker entrypoints.
pub fn api() -> Result<&'static Api, String> {
    API.get_or_init(Api::load).as_ref().map_err(Clone::clone)
}

impl Api {
    /// Loads the private native checker library and resolves exported symbols.
    fn load() -> Result<Self, String> {
        let library = Box::new(load_library(materialized_library_path()?.as_path())?);
        let library = Box::leak(library);

        Ok(Self {
            luau_checker_new: load_symbol(library, b"luau_checker_new\0")?,
            luau_checker_free: load_symbol(library, b"luau_checker_free\0")?,
            luau_cancellation_token_new: load_symbol(library, b"luau_cancellation_token_new\0")?,
            luau_cancellation_token_free: load_symbol(library, b"luau_cancellation_token_free\0")?,
            luau_cancellation_token_cancel: load_symbol(
                library,
                b"luau_cancellation_token_cancel\0",
            )?,
            luau_cancellation_token_reset: load_symbol(
                library,
                b"luau_cancellation_token_reset\0",
            )?,
            luau_checker_add_definitions: load_symbol(library, b"luau_checker_add_definitions\0")?,
            luau_checker_check: load_symbol(library, b"luau_checker_check\0")?,
            luau_extract_entrypoint_schema: load_symbol(
                library,
                b"luau_extract_entrypoint_schema\0",
            )?,
            luau_check_result_free: load_symbol(library, b"luau_check_result_free\0")?,
            luau_entrypoint_schema_result_free: load_symbol(
                library,
                b"luau_entrypoint_schema_result_free\0",
            )?,
            luau_string_free: load_symbol(library, b"luau_string_free\0")?,
        })
    }
}

/// Returns the runtime path for the materialized private native checker library.
fn materialized_library_path() -> Result<&'static PathBuf, String> {
    MATERIALIZED_LIBRARY_PATH
        .get_or_init(materialize_native_library)
        .as_ref()
        .map_err(Clone::clone)
}

/// Materializes the embedded native checker library into a stable temp-file location.
fn materialize_native_library() -> Result<PathBuf, String> {
    let content_hash = fnv1a64(NATIVE_LIBRARY_BYTES);
    let target_dir = env::temp_dir()
        .join("luau-analyze")
        .join(format!("{content_hash:016x}"));
    let target_path = target_dir.join(NATIVE_LIBRARY_FILE_NAME);

    if target_path.is_file() {
        return Ok(target_path);
    }

    fs::create_dir_all(&target_dir).map_err(|error| {
        format!(
            "failed to create native checker temp directory `{}`: {error}",
            target_dir.display()
        )
    })?;

    let temp_path = target_dir.join(format!(
        "{}.{}.tmp",
        NATIVE_LIBRARY_FILE_NAME,
        process::id()
    ));
    fs::write(&temp_path, NATIVE_LIBRARY_BYTES).map_err(|error| {
        format!(
            "failed to write embedded native checker `{}`: {error}",
            temp_path.display()
        )
    })?;

    #[cfg(unix)]
    {
        let permissions = fs::Permissions::from_mode(0o755);
        fs::set_permissions(&temp_path, permissions).map_err(|error| {
            format!(
                "failed to chmod embedded native checker `{}`: {error}",
                temp_path.display()
            )
        })?;
    }

    match fs::rename(&temp_path, &target_path) {
        Ok(()) => Ok(target_path),
        Err(_) if target_path.is_file() => {
            drop(fs::remove_file(&temp_path));
            Ok(target_path)
        }
        Err(error) => Err(format!(
            "failed to publish embedded native checker `{}`: {error}",
            target_path.display()
        )),
    }
}

/// Loads the private native checker library from disk.
fn load_library(path: &Path) -> Result<Library, String> {
    unsafe { Library::new(path) }.map_err(|error| {
        format!(
            "failed to load native checker `{}`: {error}",
            path.display()
        )
    })
}

/// Resolves one exported symbol from the private native checker library.
fn load_symbol<T>(library: &'static Library, symbol: &[u8]) -> Result<T, String>
where
    T: Copy,
{
    unsafe { library.get::<T>(symbol) }
        .map(|loaded| *loaded)
        .map_err(|error| {
            format!(
                "failed to resolve symbol `{}`: {error}",
                symbol_name(symbol)
            )
        })
}

/// Formats a NUL-terminated symbol name for diagnostics.
fn symbol_name(symbol: &[u8]) -> String {
    let bytes = symbol.strip_suffix(b"\0").unwrap_or(symbol);
    String::from_utf8_lossy(bytes).into_owned()
}

/// Computes a stable 64-bit FNV-1a hash for file naming.
fn fnv1a64(bytes: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = FNV_OFFSET;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::{NATIVE_LIBRARY_FILE_NAME, materialized_library_path};

    #[test]
    fn materialized_library_path_is_stable() {
        let path = materialized_library_path().expect("native checker should materialize");
        assert_eq!(
            path.file_name().and_then(|file_name| file_name.to_str()),
            Some(NATIVE_LIBRARY_FILE_NAME)
        );
        assert!(
            path.is_file(),
            "materialized library should exist at {path:?}"
        );
    }
}
