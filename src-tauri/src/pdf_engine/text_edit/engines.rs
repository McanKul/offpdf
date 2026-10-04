//! Engines and subprocesses (SPEC §B.8): qpdf ≥ 11, pdftoppm and pdftotext resolution, one
//! subprocess runner (argv only, capped output, timeout, cancel), `qpdf --check`
//! classification with a fingerprint memo and a background form, and qpdf's page map.

use crate::error::AppError;
use crate::models::JobHandle;
use crate::pdf_engine::text_edit::limits;
use crate::pdf_engine::text_edit::reasons::{self, EditProblem, EditProblemCode, ProblemCtx};
use crate::pdf_engine::text_edit::snapshot::Fingerprint;
use crate::pdf_engine::{qpdf, render};
use lopdf::ObjectId;
use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Engines {
    pub qpdf: PathBuf,
    pub pdftoppm: PathBuf,
    pub pdftotext: PathBuf,
}

const POPPLER_DIRS: [&str; 4] = [
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/opt/local/bin",
    "/usr/bin",
];

fn exe_name(tool: &str) -> String {
    if cfg!(windows) {
        format!("{tool}.exe")
    } else {
        tool.to_string()
    }
}

/// Poppler tool without a Tauri handle: next to the executable, the same absolute dirs as
/// `render.rs`, then the bare name (PATH).
fn poppler_standalone(tool: &str) -> PathBuf {
    let exe = exe_name(tool);
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        let candidate = dir.join("binaries").join(&exe);
        if candidate.is_file() {
            return candidate;
        }
    }
    if !cfg!(windows) {
        for dir in POPPLER_DIRS {
            let candidate = Path::new(dir).join(&exe);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    PathBuf::from(exe)
}

/// True when `exe` can be spawned: an existing file, or a bare name found on PATH.
fn tool_exists(exe: &Path) -> bool {
    if exe.components().count() > 1 || exe.is_absolute() {
        return exe.is_file();
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(exe).is_file())
}

fn tool_label(exe: &Path) -> String {
    exe.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| exe.to_string_lossy().into_owned())
}

impl Engines {
    pub fn resolve(app: &tauri::AppHandle) -> Result<Engines, AppError> {
        Engines {
            qpdf: qpdf::resolve_qpdf(app),
            pdftoppm: render::resolve_pdftoppm(app),
            pdftotext: render::resolve_pdftotext(app),
        }
        .checked()
    }

    pub fn for_export(qpdf: &Path, app: Option<&tauri::AppHandle>) -> Result<Engines, AppError> {
        let (pdftoppm, pdftotext) = match app {
            Some(app) => (
                render::resolve_pdftoppm(app),
                render::resolve_pdftotext(app),
            ),
            None => (
                poppler_standalone("pdftoppm"),
                poppler_standalone("pdftotext"),
            ),
        };
        Engines {
            qpdf: qpdf.to_path_buf(),
            pdftoppm,
            pdftotext,
        }
        .checked()
    }

    /// qpdf must run and be ≥ 11 (`ENGINE_MISSING`); both Poppler tools must exist (`VERIFIER_MISSING`).
    fn checked(self) -> Result<Engines, AppError> {
        qpdf_version_ok(&self.qpdf)?;
        for tool in [&self.pdftoppm, &self.pdftotext] {
            if !tool_exists(tool) {
                return Err(reasons::verifier_missing(&tool_label(tool)));
            }
        }
        Ok(self)
    }
}

pub struct RunOpts<'a> {
    pub handle: Option<&'a Arc<JobHandle>>,
    pub cancel: Option<&'a AtomicBool>,
    pub timeout: Duration, // default SUBPROCESS_TIMEOUT_SECS
    pub stdout_cap: usize, // stdout read is capped; excess → EDIT_VERIFY_FAILED "tool output too large"
}

impl Default for RunOpts<'_> {
    fn default() -> Self {
        RunOpts {
            handle: None,
            cancel: None,
            timeout: Duration::from_secs(limits::SUBPROCESS_TIMEOUT_SECS),
            stdout_cap: limits::PDFTOTEXT_OUTPUT_MAX,
        }
    }
}

impl<'a> RunOpts<'a> {
    fn with_stdout_cap(&self, stdout_cap: usize) -> RunOpts<'a> {
        RunOpts {
            handle: self.handle,
            cancel: self.cancel,
            timeout: self.timeout,
            stdout_cap,
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel.is_some_and(|c| c.load(Ordering::SeqCst))
            || self.handle.is_some_and(|h| h.is_cancelled())
    }
}

#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

fn verify_failed(detail: &str) -> AppError {
    let p = EditProblem::new(EditProblemCode::EditVerifyFailed, Some(detail.to_string()));
    let ctx = ProblemCtx {
        page_number: None,
        file_name: None,
        face: None,
        reason: None,
    };
    EditProblemCode::EditVerifyFailed.to_app_error(&p, &ctx)
}

/// Reads a pipe to EOF keeping at most `cap` bytes; returns (kept, overflowed).
fn read_pipe(pipe: Option<impl Read>, cap: usize) -> (Vec<u8>, bool) {
    let Some(mut pipe) = pipe else {
        return (Vec::new(), false);
    };
    let mut kept = Vec::new();
    let mut overflow = false;
    let mut buf = vec![0u8; 64 << 10];
    loop {
        match pipe.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let chunk = buf.get(..n).unwrap_or_default();
                let room = cap.saturating_sub(kept.len());
                if chunk.len() > room {
                    overflow = true;
                }
                kept.extend_from_slice(chunk.get(..room.min(chunk.len())).unwrap_or_default());
            }
        }
    }
    (kept, overflow)
}

/// Runs `exe` with `args` (never a shell string), reading stdout (capped) and stderr on helper
/// threads while polling for exit, timeout (`ENGINE_FAILED` "took too long") and cancel
/// (`CANCELLED`). A missing executable is `ENGINE_MISSING` (qpdf) or `VERIFIER_MISSING` (poppler).
/// A non-zero exit code is returned, not an error.
pub fn run_tool(
    exe: &Path,
    args: &[OsString],
    poppler: bool,
    opts: &RunOpts<'_>,
) -> Result<ToolOutput, AppError> {
    if opts.cancelled() {
        return Err(AppError::cancelled());
    }
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if poppler {
        render::configure_poppler_command(&mut cmd, exe);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(if poppler {
                reasons::verifier_missing(&tool_label(exe))
            } else {
                AppError::engine_missing()
            });
        }
        Err(e) => return Err(AppError::engine_failed(format!("{}: {e}", tool_label(exe)))),
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let cap = opts.stdout_cap;
    let out_reader = std::thread::Builder::new().spawn(move || read_pipe(stdout, cap));
    let err_reader =
        std::thread::Builder::new().spawn(move || read_pipe(stderr, limits::TOOL_STDERR_MAX));
    let (Ok(out_reader), Ok(err_reader)) = (out_reader, err_reader) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(AppError::engine_failed(
            "could not start the output readers",
        ));
    };
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(AppError::engine_failed(format!("{}: {e}", tool_label(exe))));
            }
        }
        // On timeout or cancel the readers are detached: a grandchild may still hold the pipes.
        if opts.cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(AppError::cancelled());
        }
        if started.elapsed() >= opts.timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(AppError::engine_failed(format!(
                "{} took too long and was stopped",
                tool_label(exe)
            )));
        }
        std::thread::sleep(Duration::from_millis(limits::TOOL_POLL_MS));
    };
    let (stdout, overflow) = out_reader.join().unwrap_or((Vec::new(), true));
    let (stderr, _) = err_reader.join().unwrap_or((Vec::new(), false));
    if overflow {
        return Err(verify_failed(&format!(
            "{}: tool output too large",
            tool_label(exe)
        )));
    }
    Ok(ToolOutput {
        code: status.code().unwrap_or(-1),
        stdout,
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

// ---- qpdf --check ------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum SourceCheck {
    Clean,
    Benign(Vec<String>),
    Problems(Vec<String>),
}

/// One entry of the benign-warning allow-list (each has its own ENG test; nothing else is benign).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenignRule {
    /// The lower-cased line contains this text.
    Contains(&'static str),
    /// `reported number of objects (N) is not one plus the highest object number (M)` —
    /// a wrong trailer `/Size`, which lopdf also corrects.
    SizeMismatch,
}

pub const BENIGN_CHECK_RULES: &[BenignRule] = &[
    BenignRule::Contains("linearization"),
    BenignRule::Contains("hint table"),
    BenignRule::SizeMismatch,
];

impl BenignRule {
    pub fn matches(self, line: &str) -> bool {
        let lower = line.to_ascii_lowercase();
        match self {
            BenignRule::Contains(text) => lower.contains(text),
            BenignRule::SizeMismatch => size_mismatch(&lower),
        }
    }
}

fn size_mismatch(lower: &str) -> bool {
    let digits_then = |s: &str, tail: &str| -> Option<usize> {
        let n = s.bytes().take_while(u8::is_ascii_digit).count();
        (n > 0 && s.get(n..)?.starts_with(tail)).then_some(n + tail.len())
    };
    let head = "reported number of objects (";
    let middle = ") is not one plus the highest object number (";
    let Some(p) = lower.find(head) else {
        return false;
    };
    let rest = lower.get(p + head.len()..).unwrap_or_default();
    let Some(n) = digits_then(rest, middle) else {
        return false;
    };
    digits_then(rest.get(n..).unwrap_or_default(), ")").is_some()
}

/// Warning lines of a `qpdf --check` run: every non-empty stderr line except qpdf's summary,
/// plus stdout lines starting with `WARNING`; each distinct line once, in first-seen order.
fn warning_lines(stdout: &str, stderr: &str) -> Vec<String> {
    let err = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("qpdf: operation succeeded with warnings"));
    let out = stdout
        .lines()
        .map(str::trim)
        .filter(|l| l.to_ascii_uppercase().starts_with("WARNING"));
    let mut seen = std::collections::HashSet::new();
    err.chain(out)
        .filter(|l| seen.insert(*l))
        .map(str::to_string)
        .collect()
}

/// Exit 0 → `Clean`; exit 3 → `Benign` when every warning line matches the allow-list, else
/// `Problems`; exit 2 or anything else → `Problems`.
pub fn classify_check(code: i32, stdout: &str, stderr: &str) -> SourceCheck {
    let lines = warning_lines(stdout, stderr);
    match code {
        0 => SourceCheck::Clean,
        3 if lines.is_empty() => {
            SourceCheck::Problems(vec!["qpdf reported warnings without details".to_string()])
        }
        3 if lines
            .iter()
            .all(|l| BENIGN_CHECK_RULES.iter().any(|r| r.matches(l))) =>
        {
            SourceCheck::Benign(lines)
        }
        3 => SourceCheck::Problems(lines),
        _ if lines.is_empty() => {
            SourceCheck::Problems(vec![format!("qpdf --check exited with code {code}")])
        }
        _ => SourceCheck::Problems(lines),
    }
}

pub fn qpdf_check(
    engines: &Engines,
    pdf: &Path,
    opts: &RunOpts<'_>,
) -> Result<SourceCheck, AppError> {
    #[cfg(test)]
    CHECK_RUNS.with(|c| c.set(c.get() + 1));
    let args = [OsString::from("--check"), pdf.as_os_str().to_os_string()];
    let out = run_tool(
        &engines.qpdf,
        &args,
        false,
        &opts.with_stdout_cap(limits::QPDF_JSON_MAX_BYTES),
    )?;
    // qpdf could not open the file (it vanished, e.g. "Clear temp files" during a background
    // check): that says nothing about the PDF, so it is an engine failure, never a verdict
    // (review T5 M1).
    if out.code == 2 && out.stderr.lines().any(|l| l.starts_with(QPDF_OPEN_FAILED)) {
        return Err(AppError::engine_failed(format!(
            "qpdf could not open the file to check: {}",
            out.stderr.trim()
        )));
    }
    Ok(classify_check(
        out.code,
        &String::from_utf8_lossy(&out.stdout),
        &out.stderr,
    ))
}

/// How qpdf's stderr starts when it cannot open its input file.
const QPDF_OPEN_FAILED: &str = "qpdf: open ";

static CHECK_MEMO: Mutex<VecDeque<(Fingerprint, SourceCheck)>> = Mutex::new(VecDeque::new());

/// Memoised by fingerprint (process-wide LRU of CHECK_MEMO_MAX entries): returns the stored result when the
/// same fingerprint was checked before, else runs `qpdf_check` on `pdf` (a copy written from the very bytes
/// that were fingerprinted) and stores it. Safe to share between open and Save: the key is computed from the
/// bytes qpdf reads. Errors (timeout, cancel, missing qpdf, an input qpdf could not open) are never
/// memoised, and neither is a result whose input no longer has the fingerprinted length after the run
/// (it was deleted or replaced while qpdf read it, review T5 M1).
pub fn qpdf_check_memo(
    engines: &Engines,
    fingerprint: Fingerprint,
    pdf: &Path,
    opts: &RunOpts<'_>,
) -> Result<SourceCheck, AppError> {
    {
        let mut memo = CHECK_MEMO.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pos) = memo.iter().position(|(f, _)| *f == fingerprint) {
            if let Some(entry) = memo.remove(pos) {
                let result = entry.1.clone();
                memo.push_front(entry);
                return Ok(result);
            }
        }
    }
    let result = qpdf_check(engines, pdf, opts)?;
    let still_there = std::fs::metadata(pdf).is_ok_and(|m| m.len() == fingerprint.len);
    if !still_there {
        return Ok(result);
    }
    let mut memo = CHECK_MEMO.lock().unwrap_or_else(|e| e.into_inner());
    memo.retain(|(f, _)| *f != fingerprint);
    memo.push_front((fingerprint, result.clone()));
    memo.truncate(limits::CHECK_MEMO_MAX);
    Ok(result)
}

type CheckSlot = Arc<(Mutex<Option<Result<SourceCheck, AppError>>>, Condvar)>;

/// Background form used at open: spawns a thread running `qpdf_check_memo`; `wait` blocks (honouring cancel
/// and SUBPROCESS_TIMEOUT_SECS) until the result exists.
#[derive(Clone)]
pub struct PendingCheck {
    slot: CheckSlot,
}

fn store(slot: &CheckSlot, result: Result<SourceCheck, AppError>) {
    let (lock, cv) = &**slot;
    *lock.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
    cv.notify_all();
}

impl PendingCheck {
    pub fn spawn(engines: Engines, fingerprint: Fingerprint, pdf: PathBuf) -> PendingCheck {
        let slot: CheckSlot = Arc::new((Mutex::new(None), Condvar::new()));
        let worker_slot = Arc::clone(&slot);
        let spawned = std::thread::Builder::new()
            .name("offpdf-qpdf-check".into())
            .spawn(move || {
                let result = qpdf_check_memo(&engines, fingerprint, &pdf, &RunOpts::default());
                store(&worker_slot, result);
            });
        if let Err(e) = spawned {
            store(
                &slot,
                Err(AppError::engine_failed(format!(
                    "could not start the qpdf check: {e}"
                ))),
            );
        }
        PendingCheck { slot }
    }

    pub fn wait(&self, cancel: Option<&AtomicBool>) -> Result<SourceCheck, AppError> {
        let deadline =
            Instant::now() + Duration::from_secs(limits::SUBPROCESS_TIMEOUT_SECS.saturating_add(5));
        let (lock, cv) = &*self.slot;
        let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(result) = guard.as_ref() {
                return result.clone();
            }
            if cancel.is_some_and(|c| c.load(Ordering::SeqCst)) {
                return Err(AppError::cancelled());
            }
            if Instant::now() >= deadline {
                return Err(AppError::engine_failed(
                    "qpdf --check took too long and was stopped",
                ));
            }
            guard = match cv.wait_timeout(guard, Duration::from_millis(limits::TOOL_POLL_MS)) {
                Ok((g, _)) => g,
                Err(e) => e.into_inner().0,
            };
        }
    }

    pub fn peek(&self) -> Option<Result<SourceCheck, AppError>> {
        self.slot
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

// ---- qpdf page map and version -----------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QpdfPage {
    pub object: ObjectId,
    pub contents: Vec<ObjectId>,
}

/// `"N G R"`, strictly (no signs, no leading zeros, single spaces).
fn parse_ref(s: &str) -> Option<ObjectId> {
    let mut it = s.split(' ');
    let (n, g, r) = (it.next()?, it.next()?, it.next()?);
    let canonical = |t: &str| {
        !t.is_empty() && t.bytes().all(|c| c.is_ascii_digit()) && (t == "0" || !t.starts_with('0'))
    };
    if it.next().is_some() || r != "R" || !canonical(n) || !canonical(g) {
        return None;
    }
    Some((n.parse().ok()?, g.parse().ok()?))
}

/// Parses `qpdf --json=2 --json-key=pages` output (qpdf 11.9 and 12.x shapes).
pub fn parse_qpdf_pages(json: &[u8]) -> Result<Vec<QpdfPage>, String> {
    let v: serde_json::Value =
        serde_json::from_slice(json).map_err(|e| format!("qpdf JSON: {e}"))?;
    let pages = v
        .get("pages")
        .and_then(serde_json::Value::as_array)
        .ok_or("qpdf JSON has no pages array")?;
    pages
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let n = i + 1;
            let object = p
                .get("object")
                .and_then(serde_json::Value::as_str)
                .and_then(parse_ref)
                .ok_or_else(|| format!("qpdf JSON page {n}: bad object"))?;
            let contents = p
                .get("contents")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| format!("qpdf JSON page {n}: no contents"))?
                .iter()
                .map(|c| c.as_str().and_then(parse_ref))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| format!("qpdf JSON page {n}: bad contents"))?;
            Ok(QpdfPage { object, contents })
        })
        .collect()
}

/// `qpdf --json=2 --json-key=pages <file>` (exit 0 or 3). Failures are `PDF_NEEDS_REPAIR`; the
/// gate maps them to `EDIT_VERIFY_FAILED` for files the pipeline wrote.
pub fn qpdf_page_map(
    engines: &Engines,
    pdf: &Path,
    opts: &RunOpts<'_>,
) -> Result<Vec<QpdfPage>, AppError> {
    let args = [
        OsString::from("--json=2"),
        OsString::from("--json-key=pages"),
        pdf.as_os_str().to_os_string(),
    ];
    let out = run_tool(
        &engines.qpdf,
        &args,
        false,
        &opts.with_stdout_cap(limits::QPDF_JSON_MAX_BYTES),
    )?;
    if out.code != 0 && out.code != 3 {
        let first = out.stderr.lines().next().unwrap_or_default();
        return Err(reasons::pdf_needs_repair(&[format!(
            "qpdf --json exited with code {}: {first}",
            out.code
        )]));
    }
    parse_qpdf_pages(&out.stdout).map_err(|e| reasons::pdf_needs_repair(&[e]))
}

/// Major version and version text from `qpdf --version` output ("qpdf version 12.3.2").
pub fn parse_qpdf_version(stdout: &str) -> Option<(u32, String)> {
    let line = stdout.lines().next()?.trim();
    let version = line.strip_prefix("qpdf version ")?.trim();
    let major = version.split('.').next()?.parse().ok()?;
    Some((major, version.to_string()))
}

static VERSION_CACHE: OnceLock<Mutex<HashMap<PathBuf, Result<(), AppError>>>> = OnceLock::new();

/// `qpdf --version`, major ≥ 11 (cached per path; spawn failures are not cached).
pub fn qpdf_version_ok(qpdf: &Path) -> Result<(), AppError> {
    let cache = VERSION_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(hit) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(qpdf) {
        return hit.clone();
    }
    let opts = RunOpts {
        timeout: Duration::from_secs(30),
        stdout_cap: 64 << 10,
        ..RunOpts::default()
    };
    let out = run_tool(qpdf, &[OsString::from("--version")], false, &opts)?;
    let result = match parse_qpdf_version(&String::from_utf8_lossy(&out.stdout)) {
        Some((major, _)) if major >= 11 => Ok(()),
        Some((_, version)) => Err(reasons::qpdf_too_old(&version)),
        None => Err(AppError::engine_failed(
            "OffPDF could not read the qpdf version.",
        )),
    };
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(qpdf.to_path_buf(), result.clone());
    result
}

/// Engines found without a running app (tests and benches: the dev Mac's tools).
#[cfg(test)]
impl Engines {
    pub fn standalone() -> Option<Engines> {
        Engines {
            qpdf: qpdf::resolve_qpdf_standalone(),
            pdftoppm: poppler_standalone("pdftoppm"),
            pdftotext: poppler_standalone("pdftotext"),
        }
        .checked()
        .ok()
    }
}

#[cfg(test)]
thread_local! {
    /// Test seam (ENG-08): qpdf --check runs started on this thread.
    pub(crate) static CHECK_RUNS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
