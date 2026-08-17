//! Small helpers shared by the encoder and decoder for talking to libheif's C API.

use std::ffi::CStr;
use std::ptr;
use std::sync::Once;

use crate::sys;

/// Initialize libheif exactly once before first use. libheif's objects are reference counted internally; calling
/// `heif_init` ensures plugin registration and that library memory is set up. We never call `heif_deinit` — the process
/// owns these globals for its whole lifetime, mirroring how the codec libraries expect to be used.
pub(crate) fn init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // SAFETY: runs exactly once via `Once`; passing null requests the default params.
        unsafe {
            sys::heif_init(ptr::null_mut());
        }
    });
}

/// Convert a [`heif_error`](sys::heif_error) returned by value into a `Result`. Returns `Ok(())` for `heif_error_Ok`;
/// otherwise extracts the human-readable `message`.
pub(crate) fn check(err: sys::heif_error) -> Result<(), String> {
    if err.code == sys::heif_error_code_heif_error_Ok {
        return Ok(());
    }

    // SAFETY: libheif guarantees `message` is a non-null, NUL-terminated C string for any
    // non-OK error, but we guard against null defensively.
    let message = if err.message.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(err.message).to_string_lossy().into_owned() }
    };

    Err(message)
}
