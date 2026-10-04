//! Test helpers (T1): scratch directories, the engines-or-skip policy (§E.1), fixture PDFs
//! (`pdf.rs`) and allocation counting for the peak-memory tests. Test builds only.
#![allow(dead_code)] // shared by the test modules of every task; not every helper is used by each

pub(crate) mod cff;
pub(crate) mod fakes;
pub(crate) mod fonts;
pub(crate) mod pdf;
pub(crate) mod producers;
pub(crate) mod ttf;
pub(crate) mod type1;

use crate::pdf_engine::qpdf;
use crate::pdf_engine::text_edit::engines::Engines;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicUsize, Ordering};

/// A temporary directory removed on drop.
pub struct Scratch {
    dir: PathBuf,
}

static SCRATCH_SEQ: AtomicUsize = AtomicUsize::new(0);

impl Scratch {
    pub fn new(tag: &str) -> Scratch {
        let n = SCRATCH_SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir()
            .join("offpdf-text-edit-tests")
            .join(format!("{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Scratch { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    pub fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.path(name);
        std::fs::write(&p, bytes).expect("write scratch file");
        p
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// qpdf ≥ 11, pdftoppm and pdftotext, or `None` after printing `skip: {tool} not available`.
/// With `OFFPDF_REQUIRE_ENGINES=1` a missing engine fails the test instead (§E.1).
pub fn engines_or_skip(test_name: &str) -> Option<Engines> {
    match Engines::for_export(&qpdf::resolve_qpdf_standalone(), None) {
        Ok(engines) => Some(engines),
        Err(e) => {
            let tool = match e.code.as_str() {
                "VERIFIER_MISSING" => e.details.clone().unwrap_or_else(|| "poppler".into()),
                _ => "qpdf 11+".to_string(),
            };
            if std::env::var("OFFPDF_REQUIRE_ENGINES").as_deref() == Ok("1") {
                panic!("{test_name}: {tool} not available and OFFPDF_REQUIRE_ENGINES=1 ({e})");
            }
            println!("skip: {tool} not available ({test_name})");
            None
        }
    }
}

// ---- Allocation counting -----------------------------------------------------------------
//
// A thin wrapper around the system allocator. Per-thread counting (always cheap: one TLS read)
// measures code that allocates on the calling thread (the capped inflater). Process-wide
// counting is only meaningful in a child test process that runs a single test
// (`run_child_test`), because lopdf parses on rayon worker threads.

pub struct CountingAlloc;

static GLOBAL_ON: AtomicBool = AtomicBool::new(false);
static GLOBAL_CUR: AtomicIsize = AtomicIsize::new(0);
static GLOBAL_PEAK: AtomicIsize = AtomicIsize::new(0);

thread_local! {
    static THREAD_TRACK: Cell<(bool, isize, isize)> = const { Cell::new((false, 0, 0)) };
}

fn record(delta: isize) {
    if GLOBAL_ON.load(Ordering::Relaxed) {
        let cur = GLOBAL_CUR.fetch_add(delta, Ordering::Relaxed) + delta;
        GLOBAL_PEAK.fetch_max(cur, Ordering::Relaxed);
    }
    let _ = THREAD_TRACK.try_with(|t| {
        let (on, cur, peak) = t.get();
        if on {
            let cur = cur + delta;
            t.set((on, cur, peak.max(cur)));
        }
    });
}

// SAFETY: every call is forwarded unchanged to `System`; only byte counters are updated.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded with the caller's layout.
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            record(layout.size() as isize);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded with the caller's layout.
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            record(layout.size() as isize);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` was allocated by `System` with this layout.
        unsafe { System.dealloc(ptr, layout) };
        record(-(layout.size() as isize));
    }

    /// Counted as a new buffer, then the old one freed: a moving realloc holds both while it
    /// copies, and a growing vector's peak is that overlap (review T3-budget LOW-1).
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: `ptr` was allocated by `System` with this layout.
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            record(new_size as isize);
            record(-(layout.size() as isize));
        }
        p
    }
}

#[global_allocator]
static COUNTING_ALLOC: CountingAlloc = CountingAlloc;

/// Runs `f` and returns its result with the peak number of bytes it held allocated at once on
/// the calling thread (allocations made before `f` are not counted).
pub fn thread_peak<R>(f: impl FnOnce() -> R) -> (R, usize) {
    THREAD_TRACK.with(|t| t.set((true, 0, 0)));
    let r = f();
    let (_, _, peak) = THREAD_TRACK.with(|t| t.get());
    THREAD_TRACK.with(|t| t.set((false, 0, 0)));
    (r, peak.max(0) as usize)
}

/// `thread_peak`, plus the bytes `f` left allocated on the calling thread when it returned
/// (what its result holds).
pub fn thread_peak_held<R>(f: impl FnOnce() -> R) -> (R, usize, usize) {
    THREAD_TRACK.with(|t| t.set((true, 0, 0)));
    let r = f();
    let (_, cur, peak) = THREAD_TRACK.with(|t| t.get());
    THREAD_TRACK.with(|t| t.set((false, 0, 0)));
    (r, peak.max(0) as usize, cur.max(0) as usize)
}

/// Process-wide peak of bytes held during `f` (use only inside `run_child_test` children).
pub fn process_peak<R>(f: impl FnOnce() -> R) -> (R, usize) {
    GLOBAL_CUR.store(0, Ordering::SeqCst);
    GLOBAL_PEAK.store(0, Ordering::SeqCst);
    GLOBAL_ON.store(true, Ordering::SeqCst);
    let r = f();
    GLOBAL_ON.store(false, Ordering::SeqCst);
    (r, GLOBAL_PEAK.load(Ordering::SeqCst).max(0) as usize)
}

/// Env var that turns an `#[ignore]`d child test into a real run.
pub const CHILD_ENV: &str = "OFFPDF_TEXT_EDIT_CHILD";

/// Runs one ignored test of this test binary in a fresh process (so process-wide counters see
/// only that test) and returns its stdout. `test_path` is the full test path.
pub fn run_child_test(test_path: &str, mode: &str) -> String {
    let exe = std::env::current_exe().expect("test binary path");
    let out = std::process::Command::new(exe)
        .args([
            "--exact",
            test_path,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, mode)
        .output()
        .expect("spawn child test");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "child test {test_path} failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

/// The child mode (`None` in normal runs, where child tests return immediately).
pub fn child_mode() -> Option<String> {
    std::env::var(CHILD_ENV).ok()
}

/// Parses `KEY=<number>` from child output.
pub fn child_value(stdout: &str, key: &str) -> usize {
    stdout
        .lines()
        .flat_map(str::split_whitespace)
        .find_map(|t| {
            t.strip_prefix(&format!("{key}="))
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or_else(|| panic!("{key} missing in child output:\n{stdout}"))
}
