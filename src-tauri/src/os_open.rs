//! Parse OS-opened file tokens (argv / `file://`) and hand paths to the UI.
//!
//! Paths only — this module never reads file bytes.

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, State};

/// Event the frontend listens for after it has called [`take_opened_paths`].
pub const OPENED_PATHS_EVENT: &str = "os-open:paths";

/// Cold-start buffer so Opened/argv paths are not lost before the webview listens.
#[derive(Default)]
pub struct OpenedPathQueue {
    pending: Vec<String>,
    frontend_ready: bool,
}

/// Skip argv[0], then map the rest through [`parse_opened_token`].
pub fn parse_opened_argv(args: impl IntoIterator<Item = impl AsRef<str>>) -> Vec<PathBuf> {
    let mut iter = args.into_iter();
    let _argv0 = iter.next();
    iter.filter_map(|token| parse_opened_token(token.as_ref()))
        .collect()
}

/// Accept a raw path or `file://` (percent-decoded). Drop `-flag`s, non-file
/// URLs, and bare argv[0]-like tokens. Does not read file bytes.
pub fn parse_opened_token(token: &str) -> Option<PathBuf> {
    let token = token.trim();
    if token.is_empty() || token.starts_with('-') {
        return None;
    }
    if let Some(after_scheme) = strip_file_scheme(token) {
        return file_url_to_path(after_scheme);
    }
    if has_non_file_scheme(token) {
        return None;
    }
    if is_bare_argv0_like(token) {
        return None;
    }
    Some(PathBuf::from(token))
}

/// Windows/Linux cold start: Open With arrives as argv. No-op on macOS
/// (Finder delivers `RunEvent::Opened` instead).
pub fn enqueue_cold_start_argv(app: &AppHandle) {
    #[cfg(any(windows, target_os = "linux"))]
    {
        enqueue_opened_paths(app, parse_opened_argv(std::env::args()));
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    let _ = app;
}

/// Queue paths until the frontend is ready, then emit them. No-op if empty.
pub fn enqueue_opened_paths(app: &AppHandle, paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let strings: Vec<String> = paths
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let queue = app.state::<Mutex<OpenedPathQueue>>();
    let mut guard = match queue.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    if guard.frontend_ready {
        drop(guard);
        let _ = app.emit(OPENED_PATHS_EVENT, strings);
    } else {
        guard.pending.extend(strings);
    }
}

/// Focus the existing single window (second-instance / Open With while running).
pub fn focus_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Drain queued cold-start paths and mark the frontend ready for live events.
#[tauri::command]
pub fn take_opened_paths(queue: State<'_, Mutex<OpenedPathQueue>>) -> Vec<String> {
    let mut guard = match queue.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.frontend_ready = true;
    std::mem::take(&mut guard.pending)
}

fn strip_file_scheme(token: &str) -> Option<&str> {
    // `get` returns None when 7 is not a char boundary (e.g. `/tmp/报告.pdf`).
    match token.get(..7) {
        Some(prefix) if prefix.eq_ignore_ascii_case("file://") => token.get(7..),
        _ => None,
    }
}

fn has_non_file_scheme(token: &str) -> bool {
    let Some(idx) = token.find("://") else {
        return false;
    };
    let scheme = &token[..idx];
    !scheme.is_empty()
        && scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'.' || b == b'-')
}

fn is_bare_argv0_like(token: &str) -> bool {
    !token.contains('/') && !token.contains('\\') && !token.contains('.')
}

fn file_url_to_path(after_scheme: &str) -> Option<PathBuf> {
    let path_part = strip_localhost_host(after_scheme);
    let decoded = percent_decode(path_part)?;
    if decoded.is_empty() {
        return None;
    }
    Some(PathBuf::from(windows_drive_path(decoded)))
}

fn strip_localhost_host(after_scheme: &str) -> &str {
    const HOST: &str = "localhost";
    // Off-boundary indexes (e.g. `/tmp/报告.pdf`) fall through to the raw path.
    if after_scheme
        .get(..HOST.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(HOST))
    {
        if let Some(rest) = after_scheme.get(HOST.len()..) {
            if rest.is_empty() || rest.starts_with('/') {
                return rest;
            }
        }
    }
    after_scheme
}

/// `file:///C:/Users/...` → `C:/Users/...` on Windows; unchanged elsewhere.
fn windows_drive_path(decoded: String) -> String {
    #[cfg(windows)]
    {
        if let Some(trimmed) = decoded.strip_prefix('/') {
            let bytes = trimmed.as_bytes();
            if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && (bytes[1] == b':' || bytes[1] == b'|')
            {
                let mut out = trimmed.to_string();
                if bytes[1] == b'|' {
                    out.replace_range(1..2, ":");
                }
                return out;
            }
        }
    }
    decoded
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (from_hex(bytes[i + 1]), from_hex(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
