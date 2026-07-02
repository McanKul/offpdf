//! Office document conversion via a local LibreOffice install (`soffice
//! --headless --convert-to ...`), spawned as a separate process — no network.
//!
//! LibreOffice is optional: if it isn't installed, these features are disabled
//! and the UI guides the user to install it.

use crate::error::AppError;
use crate::models::JobHandle;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use tauri::Manager;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

fn soffice_missing() -> AppError {
    AppError::new(
        "OFFICE_MISSING",
        "LibreOffice not found",
        "Converting Office documents needs LibreOffice, which couldn't be located.",
    )
    .with_suggestion("Install LibreOffice (libreoffice.org) — on macOS: brew install --cask libreoffice.")
}

/// Locate the `soffice` binary across platforms.
pub fn resolve_soffice(app: &tauri::AppHandle) -> PathBuf {
    let mut candidates: Vec<PathBuf> = Vec::new();

    for root in app_roots(app) {
        for exe in soffice_names() {
            candidates.push(root.join("libreoffice").join("program").join(exe));
            candidates.push(
                root.join("resources")
                    .join("libreoffice")
                    .join("program")
                    .join(exe),
            );
            candidates.push(
                root.join("libreoffice")
                    .join("LibreOffice")
                    .join("program")
                    .join(exe),
            );
        }
    }

    #[cfg(target_os = "macos")]
    candidates.push(PathBuf::from("/Applications/LibreOffice.app/Contents/MacOS/soffice"));
    #[cfg(windows)]
    {
        for exe in soffice_names() {
            candidates.push(PathBuf::from(format!(
                "C:\\Program Files\\LibreOffice\\program\\{exe}"
            )));
            candidates.push(PathBuf::from(format!(
                "C:\\Program Files (x86)\\LibreOffice\\program\\{exe}"
            )));
        }
    }
    for p in [
        "/opt/homebrew/bin/soffice",
        "/usr/local/bin/soffice",
        "/usr/bin/soffice",
        "/usr/bin/libreoffice",
    ] {
        candidates.push(PathBuf::from(p));
    }
    for c in candidates {
        if c.exists() {
            return c;
        }
    }
    PathBuf::from(soffice_names()[0])
}

/// Whether LibreOffice is available.
pub fn available(app: &tauri::AppHandle) -> bool {
    let exe = resolve_soffice(app);
    let mut cmd = Command::new(&exe);
    configure_soffice_command(&mut cmd, &exe);
    cmd.arg("--version").stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    matches!(cmd.status(), Ok(status) if status.success())
}

fn soffice_names() -> &'static [&'static str] {
    #[cfg(windows)]
    {
        &["soffice.com", "soffice.exe"]
    }
    #[cfg(not(windows))]
    {
        &["soffice"]
    }
}

fn app_roots(app: &tauri::AppHandle) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(res) = app.path().resource_dir() {
        roots.push(res);
    }
    if let Ok(cur) = std::env::current_exe() {
        if let Some(parent) = cur.parent() {
            roots.push(parent.to_path_buf());
        }
    }
    roots
}

fn configure_soffice_command(cmd: &mut Command, exe: &Path) {
    let Some(program_dir) = exe.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return;
    };

    cmd.current_dir(program_dir);

    if let Some(current_path) = std::env::var_os("PATH") {
        let mut paths = std::env::split_paths(&current_path).collect::<Vec<_>>();
        paths.insert(0, program_dir.to_path_buf());
        if let Ok(joined) = std::env::join_paths(paths) {
            cmd.env("PATH", joined);
        }
    } else {
        cmd.env("PATH", program_dir);
    }

}

fn hash_hex(s: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn stem_of(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "document".to_string())
}

/// Percent-encode a filesystem path for use inside a `file://` URL (RFC 3986).
/// Unreserved characters plus `/` and `:` pass through; everything else —
/// spaces, `#`, `%`, non-ASCII, … — is `%XX`-encoded. LibreOffice aborts with
/// an uncaught RuntimeException when `-env:UserInstallation` contains a raw
/// space (common on Windows, where the temp dir includes the username).
fn encode_url_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn file_url(path: &Path) -> String {
    let path = encode_url_path(&path.to_string_lossy().replace('\\', "/"));
    if cfg!(windows) {
        format!("file:///{path}")
    } else {
        format!("file://{path}")
    }
}

/// Append LibreOffice's stderr (when non-empty) to a user-facing failure
/// message, so "no export filter" style errors aren't silently swallowed.
fn with_detail(message: &str, stderr: &str) -> String {
    let detail = stderr.trim();
    if detail.is_empty() {
        message.to_string()
    } else {
        format!("{message}\n\nLibreOffice output: {detail}")
    }
}

/// Run soffice with an isolated, single-use user profile (so concurrent
/// conversions — even of the same input file — never fight over LibreOffice's
/// single-instance profile lock). Returns soffice's stderr output so callers
/// can surface it when the expected output file is missing.
fn run_soffice(
    app: &tauri::AppHandle,
    handle: Option<&Arc<JobHandle>>,
    input: &str,
    out_dir: &str,
    extra: &[&str],
) -> Result<String, AppError> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static PROFILE_SEQ: AtomicU64 = AtomicU64::new(0);

    let exe = resolve_soffice(app);
    // Unique profile per run (input + pid + counter): two conversions of the
    // same file at the same time must not share a profile, or the second one
    // fails on the profile lock. The profile is deleted after the run.
    let seq = PROFILE_SEQ.fetch_add(1, Ordering::Relaxed);
    let key = format!("{input}|{}|{seq}", std::process::id());
    let profile = std::env::temp_dir().join("offpdf-lo").join(hash_hex(&key));
    let _ = std::fs::create_dir_all(&profile);
    let user_install = format!("-env:UserInstallation={}", file_url(&profile));

    let mut cmd = Command::new(&exe);
    configure_soffice_command(&mut cmd, &exe);
    cmd.arg("--headless").arg(user_install);
    for a in extra {
        cmd.arg(a);
    }
    cmd.arg("--outdir").arg(out_dir).arg(input);
    cmd.stdout(Stdio::null()).stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);

    let result = (move || -> Result<String, AppError> {
        // When run inside a job, register the child so cancel_job can kill it.
        if let Some(h) = handle {
            let (status, stderr) = match crate::utils::process::run_tracked(h, cmd) {
                Ok(v) => v,
                Err(e) if e.code == "ENGINE_MISSING" => return Err(soffice_missing()),
                Err(e) => return Err(e),
            };
            if !status.map(|s| s.success()).unwrap_or(false) {
                return Err(AppError::engine_failed(stderr.trim().to_string()));
            }
            return Ok(stderr);
        }

        let out = cmd.output().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                soffice_missing()
            } else {
                AppError::engine_failed(e.to_string())
            }
        })?;
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        if !out.status.success() {
            return Err(AppError::engine_failed(stderr.trim().to_string()));
        }
        Ok(stderr)
    })();

    // Single-use profile: clean up regardless of outcome.
    let _ = std::fs::remove_dir_all(&profile);
    result
}

/// Convert an Office document to a PDF written into `out_dir`. Returns the path.
pub fn to_pdf(
    app: &tauri::AppHandle,
    handle: Option<&Arc<JobHandle>>,
    input: &str,
    out_dir: &str,
) -> Result<String, AppError> {
    if !Path::new(input).is_file() {
        return Err(AppError::new(
            "INVALID_INPUT",
            "File not found",
            format!("\"{input}\" could not be read."),
        ));
    }
    std::fs::create_dir_all(out_dir).map_err(|_| AppError::output_not_writable(out_dir))?;
    let stderr = run_soffice(app, handle, input, out_dir, &["--convert-to", "pdf"])?;
    let expected = Path::new(out_dir).join(format!("{}.pdf", stem_of(input)));
    if !expected.exists() {
        return Err(AppError::engine_failed(with_detail(
            "LibreOffice did not produce a PDF (the document may be unsupported or corrupt).",
            &stderr,
        )));
    }
    Ok(expected.to_string_lossy().to_string())
}

/// Convert a PDF to PDF/A-2b in `out_dir` (re-exported via LibreOffice). Returns
/// the produced path (`out_dir/<input stem>.pdf`).
pub fn to_pdfa(
    app: &tauri::AppHandle,
    handle: Option<&Arc<JobHandle>>,
    input_pdf: &str,
    out_dir: &str,
) -> Result<String, AppError> {
    if !Path::new(input_pdf).is_file() {
        return Err(AppError::invalid_pdf(input_pdf));
    }
    std::fs::create_dir_all(out_dir).map_err(|_| AppError::output_not_writable(out_dir))?;
    let stderr = run_soffice(
        app,
        handle,
        input_pdf,
        out_dir,
        &["--convert-to", "pdf:writer_pdf_Export:{\"SelectPdfVersion\":{\"type\":\"long\",\"value\":\"2\"}}"],
    )?;
    let expected = Path::new(out_dir).join(format!("{}.pdf", stem_of(input_pdf)));
    if !expected.exists() {
        return Err(AppError::engine_failed(with_detail(
            "LibreOffice could not produce a PDF/A file.",
            &stderr,
        )));
    }
    Ok(expected.to_string_lossy().to_string())
}

/// Convert a PDF to an editable Office format ("docx", "pptx", "xlsx") in
/// `out_dir`. Best effort — layout fidelity varies. Returns the path.
pub fn from_pdf(
    app: &tauri::AppHandle,
    handle: Option<&Arc<JobHandle>>,
    input_pdf: &str,
    out_dir: &str,
    target_ext: &str,
) -> Result<String, AppError> {
    if !Path::new(input_pdf).is_file() {
        return Err(AppError::invalid_pdf(input_pdf));
    }
    std::fs::create_dir_all(out_dir).map_err(|_| AppError::output_not_writable(out_dir))?;

    let (filter, ext) = match target_ext {
        "docx" => ("writer_pdf_import", "docx"),
        "pptx" => ("impress_pdf_import", "pptx"),
        // LibreOffice has no PDF import filter for Calc: the PDF loads into
        // Draw, which has no xlsx export, so this can never succeed.
        "xlsx" => {
            return Err(AppError::new(
                "BAD_FORMAT",
                "Excel export not supported",
                "LibreOffice cannot convert a PDF into an Excel workbook.",
            )
            .with_suggestion(
                "Convert to Word (.docx) instead, then copy the tables into Excel.",
            ))
        }
        other => {
            return Err(AppError::new(
                "BAD_FORMAT",
                "Unsupported format",
                format!("Cannot convert to \"{other}\"."),
            ))
        }
    };

    let stderr = run_soffice(
        app,
        handle,
        input_pdf,
        out_dir,
        &[&format!("--infilter={filter}"), "--convert-to", ext],
    )?;
    let expected = Path::new(out_dir).join(format!("{}.{ext}", stem_of(input_pdf)));
    if !expected.exists() {
        return Err(AppError::engine_failed(with_detail(
            "LibreOffice could not convert this PDF to the chosen format.",
            &stderr,
        )));
    }
    Ok(expected.to_string_lossy().to_string())
}
