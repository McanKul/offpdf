//! ENG-01…09: tool resolution, the subprocess runner, `qpdf --check` classification and memo,
//! the background check and qpdf's page map.

use crate::error::AppError;
use crate::models::JobRegistry;
use crate::pdf_engine::text_edit::engines::{
    classify_check, parse_qpdf_pages, parse_qpdf_version, qpdf_check, qpdf_check_memo,
    qpdf_page_map, qpdf_version_ok, run_tool, BenignRule, Engines, PendingCheck, QpdfPage, RunOpts,
    SourceCheck, BENIGN_CHECK_RULES, CHECK_RUNS,
};
use crate::pdf_engine::text_edit::snapshot::{check_page_map, read_snapshot, Fingerprint};
use crate::pdf_engine::text_edit::testkit::pdf::{simple_pdf, Doc};
use crate::pdf_engine::text_edit::testkit::{engines_or_skip, Scratch};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

fn runs() -> usize {
    CHECK_RUNS.with(|c| c.get())
}

#[cfg(unix)]
fn script(s: &Scratch, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let p = s.write(name, format!("#!/bin/sh\n{body}\n").as_bytes());
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

#[test]
fn eng01_qpdf_version() {
    assert_eq!(
        parse_qpdf_version("qpdf version 12.3.2\nRun qpdf --copyright"),
        Some((12, "12.3.2".into())),
        "ENG-01"
    );
    assert_eq!(
        parse_qpdf_version("qpdf version 11.9.0\n"),
        Some((11, "11.9.0".into()))
    );
    assert_eq!(
        parse_qpdf_version("qpdf version 10.6.3"),
        Some((10, "10.6.3".into()))
    );
    assert_eq!(parse_qpdf_version("something else"), None);
    assert_eq!(parse_qpdf_version(""), None);
    if let Some(engines) = engines_or_skip("eng01_qpdf_version") {
        assert!(
            qpdf_version_ok(&engines.qpdf).is_ok(),
            "ENG-01 installed qpdf ≥ 11"
        );
    }
    let err = qpdf_version_ok(Path::new("/nonexistent/offpdf/qpdf")).unwrap_err();
    assert_eq!(err.code, "ENGINE_MISSING", "missing qpdf");
    #[cfg(unix)]
    {
        let s = Scratch::new("eng01");
        let old = script(&s, "qpdf-old", "echo 'qpdf version 10.6.3'");
        let e = qpdf_version_ok(&old).unwrap_err();
        assert_eq!(e.code, "ENGINE_MISSING", "ENG-01 qpdf < 11");
        assert!(e.details.unwrap().contains("qpdf 11 or newer is required"));
        let odd = script(&s, "qpdf-odd", "echo 'not qpdf'");
        assert_eq!(qpdf_version_ok(&odd).unwrap_err().code, "ENGINE_FAILED");
        assert_eq!(
            Engines::for_export(&old, None).unwrap_err().code,
            "ENGINE_MISSING"
        );
    }
}

#[cfg(unix)]
#[test]
fn eng02_timeout_kills_a_sleeping_child() {
    let opts = RunOpts {
        timeout: Duration::from_millis(300),
        ..RunOpts::default()
    };
    let t = Instant::now();
    let e = run_tool(Path::new("/bin/sleep"), &os(&["5"]), false, &opts).unwrap_err();
    assert_eq!(e.code, "ENGINE_FAILED", "ENG-02");
    assert!(e.details.unwrap().contains("took too long"));
    assert!(
        t.elapsed() < Duration::from_secs(3),
        "ENG-02 killed, not waited for"
    );
}

#[cfg(unix)]
#[test]
fn eng03_cancel_kills() {
    let cancel = AtomicBool::new(false);
    let opts = RunOpts {
        cancel: Some(&cancel),
        ..RunOpts::default()
    };
    let t = Instant::now();
    let e = std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(Duration::from_millis(200));
            cancel.store(true, Ordering::SeqCst);
        });
        run_tool(Path::new("/bin/sleep"), &os(&["5"]), false, &opts).unwrap_err()
    });
    assert_eq!(e.code, "CANCELLED", "ENG-03");
    assert!(t.elapsed() < Duration::from_secs(3));
    let registry = JobRegistry::default();
    let handle = registry.register("eng03");
    handle.cancel();
    let opts = RunOpts {
        handle: Some(&handle),
        ..RunOpts::default()
    };
    assert_eq!(
        run_tool(Path::new("/bin/sleep"), &os(&["5"]), false, &opts)
            .unwrap_err()
            .code,
        "CANCELLED",
        "job handle"
    );
}

#[cfg(unix)]
#[test]
fn eng04_stdout_cap_and_missing_tools() {
    let capped = RunOpts {
        stdout_cap: 1_000,
        ..RunOpts::default()
    };
    let e = run_tool(
        Path::new("/usr/bin/head"),
        &os(&["-c", "200000", "/dev/zero"]),
        false,
        &capped,
    )
    .unwrap_err();
    assert_eq!(e.code, "EDIT_VERIFY_FAILED", "ENG-04");
    assert!(e.details.unwrap().contains("tool output too large"));
    let ok = run_tool(
        Path::new("/usr/bin/head"),
        &os(&["-c", "1000", "/dev/zero"]),
        false,
        &capped,
    )
    .unwrap();
    assert_eq!(
        (ok.code, ok.stdout.len()),
        (0, 1000),
        "ENG-04 exactly the cap is fine"
    );
    let failing = run_tool(
        Path::new("/bin/sh"),
        &os(&["-c", "echo oops >&2; exit 2"]),
        false,
        &RunOpts::default(),
    )
    .unwrap();
    assert_eq!(
        (failing.code, failing.stderr.trim()),
        (2, "oops"),
        "non-zero exit is returned, not an error"
    );
    let missing = Path::new("/nonexistent/offpdf/tool");
    assert_eq!(
        run_tool(missing, &[], false, &RunOpts::default())
            .unwrap_err()
            .code,
        "ENGINE_MISSING"
    );
    assert_eq!(
        run_tool(missing, &[], true, &RunOpts::default())
            .unwrap_err()
            .code,
        "VERIFIER_MISSING"
    );
    let e = Engines::for_export(&crate::pdf_engine::qpdf::resolve_qpdf_standalone(), None);
    if let Err(e) = e {
        assert!(matches!(
            e.code.as_str(),
            "ENGINE_MISSING" | "VERIFIER_MISSING"
        ));
    }
}

#[test]
fn eng05_check_classification_allow_list() {
    assert_eq!(
        classify_check(0, "checking a.pdf\nPDF Version: 1.7\n", ""),
        SourceCheck::Clean,
        "ENG-05 exit 0"
    );
    let lin = "WARNING: a.pdf: linearization data is inconsistent";
    let hint = "WARNING: a.pdf: page 0: shared object 12: in hint table but not computed list";
    let summary = "qpdf: operation succeeded with warnings";
    assert_eq!(
        classify_check(3, "", &format!("{lin}\n{summary}\n")),
        SourceCheck::Benign(vec![lin.into()]),
        "ENG-05 linearization"
    );
    assert_eq!(
        classify_check(3, &format!("{hint}\n"), summary),
        SourceCheck::Benign(vec![hint.into()]),
        "ENG-05 hint table (stdout)"
    );
    let other = "WARNING: a.pdf (object 5 0): expected endobj";
    assert_eq!(
        classify_check(3, "", &format!("{lin}\n{other}\n")),
        SourceCheck::Problems(vec![lin.into(), other.into()]),
        "ENG-05 any other warning"
    );
    assert_eq!(
        classify_check(
            3,
            &format!("{other}\n"),
            &format!("{other}\n{lin}\n{other}\n")
        ),
        SourceCheck::Problems(vec![other.into(), lin.into()]),
        "ENG-05 each distinct line once, first-seen order (stderr then stdout)"
    );
    assert_eq!(
        classify_check(3, "", summary),
        SourceCheck::Problems(vec!["qpdf reported warnings without details".into()])
    );
    assert_eq!(
        classify_check(2, "", "qpdf: a.pdf: not a PDF file"),
        SourceCheck::Problems(vec!["qpdf: a.pdf: not a PDF file".into()])
    );
    assert_eq!(
        classify_check(-1, "", ""),
        SourceCheck::Problems(vec!["qpdf --check exited with code -1".into()])
    );
    // one case per allow-list entry, and nothing else is benign
    assert_eq!(BENIGN_CHECK_RULES.len(), 3);
    assert!(
        BenignRule::Contains("linearization").matches("WARNING: LINEARIZATION dictionary broken")
    );
    assert!(BenignRule::Contains("hint table").matches("... Hint Table ..."));
    assert!(!BenignRule::Contains("hint table").matches("WARNING: xref table damaged"));
}

#[test]
fn eng06_size_mismatch_is_benign() {
    let line = "WARNING: a.pdf: reported number of objects (3) is not one plus the highest object number (11)";
    assert!(BenignRule::SizeMismatch.matches(line), "ENG-06");
    assert!(!BenignRule::SizeMismatch.matches(
        "WARNING: reported number of objects () is not one plus the highest object number (11)"
    ));
    assert!(!BenignRule::SizeMismatch.matches("WARNING: reported number of objects (3) is wrong"));
    assert_eq!(
        classify_check(3, "", line),
        SourceCheck::Benign(vec![line.into()])
    );
    let Some(engines) = engines_or_skip("eng06_size_mismatch_is_benign") else {
        return;
    };
    let s = Scratch::new("eng06");
    let good = simple_pdf(b"q Q");
    let i = good.windows(6).rposition(|w| w == b"/Size ").unwrap() + 6;
    let j = i + good[i..].iter().position(|c| *c == b' ').unwrap();
    let bad = [&good[..i], b"2", &good[j..]].concat();
    let p = s.write("badsize.pdf", &bad);
    match qpdf_check(&engines, &p, &RunOpts::default()).unwrap() {
        SourceCheck::Benign(lines) => assert!(
            lines
                .iter()
                .all(|l| l.contains("reported number of objects")),
            "ENG-06 {lines:?}"
        ),
        other => panic!("ENG-06 real qpdf on a wrong /Size: {other:?}"),
    }
    assert_eq!(
        qpdf_check(
            &engines,
            &s.write("ok.pdf", &simple_pdf(b"q Q")),
            &RunOpts::default()
        )
        .unwrap(),
        SourceCheck::Clean
    );
}

const QPDF12: &str = r#"{"version":2,"parameters":{"decodelevel":"generalized"},"pages":[{"contents":["4 0 R","5 0 R"],"images":[],"label":null,"object":"3 0 R","outlines":[],"pageposfrom1":1},{"contents":[],"images":[],"label":null,"object":"10 0 R","outlines":[],"pageposfrom1":2}]}"#;
const QPDF119: &str = r#"{"version": 2, "parameters": {"decodelevel": "generalized"}, "pages": [{"contents": ["4 0 R"], "images": [{"object": "8 0 R", "width": 1}], "label": {"index": 0}, "object": "3 0 R", "outlines": [{"object": "12 0 R"}], "pageposfrom1": 1}]}"#;

#[test]
fn eng07_page_map_json_shapes() {
    let pages = parse_qpdf_pages(QPDF12.as_bytes()).unwrap();
    assert_eq!(
        pages,
        [
            QpdfPage {
                object: (3, 0),
                contents: vec![(4, 0), (5, 0)]
            },
            QpdfPage {
                object: (10, 0),
                contents: vec![]
            }
        ],
        "ENG-07 qpdf 12.x"
    );
    assert_eq!(
        parse_qpdf_pages(QPDF119.as_bytes()).unwrap(),
        [QpdfPage {
            object: (3, 0),
            contents: vec![(4, 0)]
        }],
        "ENG-07 qpdf 11.9"
    );
    for bad in [
        r#"{"pages":[{"object":"3 0 obj","contents":[]}]}"#,
        r#"{"pages":[{"object":"03 0 R","contents":[]}]}"#,
        r#"{"pages":[{"object":"3  0 R","contents":[]}]}"#,
        r#"{"pages":[{"object":"3 0 R"}]}"#,
        r#"{"pages":[{"object":"3 0 R","contents":["x"]}]}"#,
        r#"{"pages":[{"object":"4294967296 0 R","contents":[]}]}"#,
        r#"{"version":2}"#,
        "not json",
    ] {
        assert!(
            parse_qpdf_pages(bad.as_bytes()).is_err(),
            "ENG-07 rejects {bad}"
        );
    }
    let Some(engines) = engines_or_skip("eng07_page_map_json_shapes") else {
        return;
    };
    let s = Scratch::new("eng07");
    let d = Doc::new(&[&[b"q", b"Q"], &[b"q Q"], &[]]);
    let p = s.write("three.pdf", &d.build());
    let pages = qpdf_page_map(&engines, &p, &RunOpts::default()).unwrap();
    let want: Vec<QpdfPage> = d
        .page_ids
        .iter()
        .zip(&d.content_ids)
        .map(|(pg, c)| QpdfPage {
            object: (*pg, 0),
            contents: c.iter().map(|i| (*i, 0)).collect(),
        })
        .collect();
    assert_eq!(pages, want, "ENG-07 installed qpdf");
    assert!(check_page_map(&read_snapshot(&p).unwrap(), &pages).is_ok());
    assert_eq!(
        qpdf_page_map(&engines, &s.write("junk.pdf", b"junk"), &RunOpts::default())
            .unwrap_err()
            .code,
        "PDF_NEEDS_REPAIR"
    );
}

/// A fingerprint numbered `n` for the file at `p`: its real length (the memo keeps only results
/// whose input still has the fingerprinted length, review T5 M1) and a made-up hash.
fn fp_of(p: &Path, n: u64) -> Fingerprint {
    Fingerprint {
        len: std::fs::metadata(p).map_or(0, |m| m.len()),
        fnv: 0xE7_0000_0000 + n,
    }
}

#[test]
fn eng08_check_memo() {
    let Some(engines) = engines_or_skip("eng08_check_memo") else {
        return;
    };
    let s = Scratch::new("eng08");
    let p = s.write("a.pdf", &simple_pdf(b"q Q"));
    let before = runs();
    assert_eq!(
        qpdf_check_memo(&engines, fp_of(&p, 1), &p, &RunOpts::default()).unwrap(),
        SourceCheck::Clean
    );
    assert_eq!(
        qpdf_check_memo(&engines, fp_of(&p, 1), &p, &RunOpts::default()).unwrap(),
        SourceCheck::Clean
    );
    assert_eq!(
        runs() - before,
        1,
        "ENG-08 the second call is served from the memo without spawning"
    );
    let broken = Engines {
        qpdf: PathBuf::from("/nonexistent/offpdf/qpdf"),
        ..engines.clone()
    };
    let before = runs();
    for _ in 0..2 {
        assert_eq!(
            qpdf_check_memo(&broken, fp_of(&p, 2), &p, &RunOpts::default())
                .unwrap_err()
                .code,
            "ENGINE_MISSING"
        );
    }
    assert_eq!(runs() - before, 2, "ENG-08 errors are never memoised");
    for n in 10..(10 + crate::pdf_engine::text_edit::limits::CHECK_MEMO_MAX as u64) {
        qpdf_check_memo(&engines, fp_of(&p, n), &p, &RunOpts::default()).unwrap();
    }
    let before = runs();
    qpdf_check_memo(&engines, fp_of(&p, 1), &p, &RunOpts::default()).unwrap();
    assert_eq!(
        runs() - before,
        1,
        "ENG-08 least recently used entry evicted"
    );
}

/// Review T5 M1: a check whose input vanished before qpdf opened it is an engine failure, never
/// memoised; the same bytes checked again later get their real verdict (it used to be
/// `PDF_NEEDS_REPAIR` for every later preview and Save of that file).
#[test]
fn eng08b_a_vanished_input_is_never_a_memoised_verdict() {
    let Some(engines) = engines_or_skip("eng08b_a_vanished_input_is_never_a_memoised_verdict")
    else {
        return;
    };
    let s = Scratch::new("eng08b");
    let p = s.write("a.pdf", &simple_pdf(b"q Q"));
    let key = fp_of(&p, 0xB0);
    let gone = s.path("gone").join("source.pdf");
    let missing = qpdf_check_memo(&engines, key, &gone, &RunOpts::default());
    assert_eq!(
        missing.map_err(|e| e.code),
        Err("ENGINE_FAILED".to_string()),
        "a missing input is no verdict"
    );
    assert_eq!(
        qpdf_check_memo(&engines, key, &p, &RunOpts::default()).unwrap(),
        SourceCheck::Clean,
        "the good copy is checked, not served a memoised open failure"
    );
    // A run whose input no longer has the fingerprinted length is returned but not kept.
    let moved = s.write("b.pdf", &simple_pdf(b"q Q"));
    let other = Fingerprint {
        len: key.len + 1,
        fnv: key.fnv + 1,
    };
    let before = runs();
    for _ in 0..2 {
        qpdf_check_memo(&engines, other, &moved, &RunOpts::default()).unwrap();
    }
    assert_eq!(runs() - before, 2, "a length mismatch is never memoised");
}

#[test]
fn eng09_pending_check_waits_and_honours_cancel() {
    let Some(engines) = engines_or_skip("eng09_pending_check_waits_and_honours_cancel") else {
        return;
    };
    let s = Scratch::new("eng09");
    let p = s.write("a.pdf", &simple_pdf(b"q Q"));
    let pending = PendingCheck::spawn(engines.clone(), fp_of(&p, 100), p.clone());
    assert_eq!(pending.wait(None).unwrap(), SourceCheck::Clean, "ENG-09");
    assert_eq!(
        pending.peek().map(|r| r.map_err(|e| e.code)),
        Some(Ok(SourceCheck::Clean))
    );
    let missing = PendingCheck::spawn(
        Engines {
            qpdf: PathBuf::from("/nonexistent/qpdf"),
            ..engines.clone()
        },
        fp_of(&p, 101),
        p.clone(),
    );
    assert_eq!(
        missing.wait(None).map_err(|e: AppError| e.code),
        Err("ENGINE_MISSING".into())
    );
    #[cfg(unix)]
    {
        let slow = script(&s, "qpdf-slow", "exec sleep 3");
        let pending = PendingCheck::spawn(
            Engines {
                qpdf: slow,
                ..engines
            },
            fp_of(&p, 102),
            p,
        );
        let cancel = AtomicBool::new(true);
        let t = Instant::now();
        assert_eq!(
            pending.wait(Some(&cancel)).unwrap_err().code,
            "CANCELLED",
            "ENG-09 cancel"
        );
        assert!(t.elapsed() < Duration::from_secs(1));
        assert!(pending.peek().is_none(), "still running in the background");
    }
    // signatures production uses but tests cannot call without a Tauri app
    let _resolve: fn(&tauri::AppHandle) -> Result<Engines, AppError> = Engines::resolve;
    let _standalone: fn() -> Option<Engines> = Engines::standalone;
}
