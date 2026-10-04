//! SNAP-01…16: one bounded read, preflight, guarded lopdf load, policy, page-map agreement.

use crate::error::AppError;
use crate::pdf_engine::text_edit::engines::{qpdf_page_map, run_tool, RunOpts};
use crate::pdf_engine::text_edit::limits;
use crate::pdf_engine::text_edit::snapshot::{
    check_page_map, fnv1a_u64, read_snapshot, read_verification_snapshot, snapshot_from_bytes,
    stat_matches, Fingerprint, SourceSnapshot,
};
use crate::pdf_engine::text_edit::testkit::pdf::{simple_pdf, zlib_zero_bomb, Doc, XrefStyle};
use crate::pdf_engine::text_edit::testkit::{
    child_mode, child_value, engines_or_skip, process_peak, run_child_test, thread_peak, Scratch,
};
use crate::pdf_engine::validate_output::content_digest;
use lopdf::Object;
use std::ffi::OsString;
use std::path::Path;

fn code(r: &Result<SourceSnapshot, AppError>) -> String {
    match r {
        Ok(_) => "OK".to_string(),
        Err(e) => e.code.clone(),
    }
}

fn from_bytes(bytes: &[u8]) -> Result<SourceSnapshot, AppError> {
    snapshot_from_bytes(Path::new("fixture.pdf"), bytes.to_vec(), None)
}

fn ok(r: Result<SourceSnapshot, AppError>, id: &str) -> SourceSnapshot {
    r.unwrap_or_else(|e| panic!("{id}: {e} ({:?})", e.details))
}

const HELLO: &[u8] = b"BT /F1 12 Tf 72 720 Td (Hello) Tj ET";

#[test]
fn snap01_single_read_and_fingerprint() {
    let s = Scratch::new("snap01");
    let bytes = simple_pdf(HELLO);
    let p = s.write("a.pdf", &bytes);
    let snap = ok(read_snapshot(&p), "SNAP-01");
    assert_eq!(
        snap.bytes.as_slice(),
        &bytes[..],
        "SNAP-01 bytes are the one read"
    );
    assert_eq!(
        snap.fingerprint,
        Fingerprint {
            len: bytes.len() as u64,
            fnv: fnv1a_u64(&bytes)
        },
        "SNAP-01"
    );
    assert_eq!(
        snap.fingerprint.fnv,
        content_digest(&bytes).hash,
        "same FNV-1a as validate_output"
    );
    assert_eq!(snap.pages.len(), 1);
    assert_eq!(
        snap.modified,
        std::fs::metadata(&p).unwrap().modified().ok()
    );
    let text = snap.fingerprint.to_string();
    assert_eq!(
        text.parse::<Fingerprint>(),
        Ok(snap.fingerprint),
        "SNAP-01 strict inverse"
    );
    assert_eq!(
        Fingerprint { len: 0, fnv: 1 }.to_string(),
        "0000000000000001-0"
    );
    for bad in [
        "",
        "0123",
        "ABCDEF0123456789-1f",
        "0123456789abcdef-01",
        "0123456789abcdef-",
        "0123456789abcdef",
        "0123456789abcdef-1-2",
        "0123456789abcdeg-1",
        " 0123456789abcdef-1",
    ] {
        assert!(
            bad.parse::<Fingerprint>().is_err(),
            "SNAP-01 rejects {bad:?}"
        );
    }
    assert_eq!(
        ok(from_bytes(&bytes), "SNAP-01").fingerprint,
        snap.fingerprint
    );
}

#[test]
fn snap02_over_the_cap_is_file_too_large() {
    let s = Scratch::new("snap02");
    let p = s.path("sparse.pdf");
    std::fs::File::create(&p)
        .unwrap()
        .set_len(limits::FILE_CAP_BYTES + 1)
        .unwrap();
    assert_eq!(
        code(&read_snapshot(&p)),
        "FILE_TOO_LARGE",
        "SNAP-02 400 MiB + 1 (sparse)"
    );
    let small = s.write("small.pdf", &simple_pdf(HELLO));
    limits::set_file_cap_override(Some(100));
    let (a, b) = (
        code(&read_snapshot(&small)),
        code(&from_bytes(&simple_pdf(HELLO))),
    );
    limits::set_file_cap_override(None);
    assert_eq!(
        (a.as_str(), b.as_str()),
        ("FILE_TOO_LARGE", "FILE_TOO_LARGE"),
        "SNAP-02 cap override"
    );
    assert_eq!(code(&read_snapshot(&s.path("missing.pdf"))), "INVALID_PDF");
    assert_eq!(
        code(&read_snapshot(s.dir())),
        "INVALID_PDF",
        "a directory is not a file"
    );
}

#[test]
fn snap03_rewritten_file_has_a_different_fingerprint() {
    let s = Scratch::new("snap03");
    let p = s.write("a.pdf", &simple_pdf(HELLO));
    let before = ok(read_snapshot(&p), "SNAP-03");
    std::fs::write(&p, simple_pdf(b"BT /F1 12 Tf 72 720 Td (Hellp) Tj ET")).unwrap();
    let after = ok(read_snapshot(&p), "SNAP-03");
    assert_eq!(before.fingerprint.len, after.fingerprint.len);
    assert_ne!(before.fingerprint, after.fingerprint, "SNAP-03");
    assert_eq!(code(&from_bytes(b"not a pdf at all")), "INVALID_PDF");
}

#[test]
fn snap04_encrypted_is_refused() {
    let d = Doc::new(&[&[HELLO]]);
    let direct = d.b.build(&format!(
        "{} /Encrypt << /Filter /Standard /V 1 /R 2 /P -4 >>",
        d.trailer()
    ));
    assert_eq!(
        code(&from_bytes(&direct)),
        "ENCRYPTED",
        "SNAP-04 direct /Encrypt dict"
    );
    let mut d2 = Doc::new(&[&[HELLO]]);
    let enc = d2.b.add("<< /Filter /Standard /V 1 /R 2 /P -4 >>");
    let indirect = d2.b.build(&format!("{} /Encrypt {enc} 0 R", d2.trailer()));
    assert_eq!(
        code(&from_bytes(&indirect)),
        "ENCRYPTED",
        "SNAP-04 indirect /Encrypt"
    );
    let Some(engines) = engines_or_skip("snap04_encrypted_is_refused") else {
        return;
    };
    let s = Scratch::new("snap04");
    let plain = s.write("plain.pdf", &simple_pdf(HELLO));
    let out = s.path("enc.pdf");
    let args: Vec<OsString> = ["--encrypt", "user", "owner", "256", "--"]
        .iter()
        .map(OsString::from)
        .chain([plain.into(), out.clone().into()])
        .collect();
    let r = run_tool(&engines.qpdf, &args, false, &RunOpts::default()).unwrap();
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(
        code(&read_snapshot(&out)),
        "ENCRYPTED",
        "SNAP-04 qpdf --encrypt (FX-ENC)"
    );
}

fn with_catalog(extra: &str, add: impl FnOnce(&mut Doc) -> String) -> Vec<u8> {
    let mut d = Doc::new(&[&[HELLO]]);
    let more = add(&mut d);
    d.b.set(
        d.catalog,
        format!("<< /Type /Catalog /Pages {} 0 R {extra} {more} >>", d.pages),
    );
    d.build()
}

#[test]
fn snap05_applied_signature_is_signed_empty_widget_is_fine() {
    let signed = with_catalog("", |d| {
        let sig = d
            .b
            .add("<< /Type /Sig /Filter /Adobe.PPKLite /ByteRange [0 10 20 30] /Contents <00> >>");
        let field = d.b.add(format!(
            "<< /FT /Sig /T (s) /V {sig} 0 R /Subtype /Widget /Rect [0 0 0 0] >>"
        ));
        format!("/AcroForm << /Fields [{field} 0 R] /SigFlags 3 >>")
    });
    assert_eq!(
        code(&from_bytes(&signed)),
        "SIGNED",
        "SNAP-05 applied signature"
    );
    let empty = with_catalog("", |d| {
        let field =
            d.b.add("<< /FT /Sig /T (s) /Subtype /Widget /Rect [0 0 0 0] >>");
        format!("/AcroForm << /Fields [{field} 0 R] >>")
    });
    assert_eq!(
        code(&from_bytes(&empty)),
        "OK",
        "SNAP-05 empty signature widget"
    );
    let perms = with_catalog("/Perms << /DocMDP << /Type /SigRef >> >>", |_| {
        String::new()
    });
    assert_eq!(code(&from_bytes(&perms)), "SIGNED", "SNAP-05 /Perms");
}

#[test]
fn snap06_xfa_is_refused() {
    let xfa = with_catalog("", |d| {
        let x = d.b.add_stream("", b"<xdp:xdp/>");
        format!("/AcroForm << /Fields [] /XFA {x} 0 R >>")
    });
    assert_eq!(code(&from_bytes(&xfa)), "UNSUPPORTED_XFA", "SNAP-06");
    let needs = with_catalog("/NeedsRendering true", |_| String::new());
    assert_eq!(
        code(&from_bytes(&needs)),
        "UNSUPPORTED_XFA",
        "SNAP-06 NeedsRendering"
    );
    let plain_form = with_catalog("/AcroForm << /Fields [] /XFA null >>", |_| String::new());
    assert_eq!(code(&from_bytes(&plain_form)), "OK");
}

fn objstm_bomb_pdf() -> Vec<u8> {
    let mut d = Doc::new(&[&[HELLO]]);
    let bomb = zlib_zero_bomb(1024);
    d.b.add_stream("/Type /ObjStm /N 1 /First 4 /Filter /FlateDecode", &bomb);
    d.build()
}

#[test]
fn snap07_objstm_bomb_is_file_too_complex_with_bounded_memory() {
    assert_eq!(
        code(&from_bytes(&objstm_bomb_pdf())),
        "FILE_TOO_COMPLEX",
        "SNAP-07"
    );
    let out = run_child_test(
        "pdf_engine::text_edit::tests_io::snap::snap07_child",
        "snap07",
    );
    let (file, peak) = (child_value(&out, "FILE"), child_value(&out, "PEAK"));
    assert_eq!(child_value(&out, "TOO_COMPLEX"), 1, "SNAP-07 child");
    assert!(
        peak < file + (48 << 20),
        "SNAP-07 peak {peak} for a 1 GiB bomb in a {file}-byte file"
    );
}

#[test]
#[ignore = "child process of snap07 (run by it)"]
fn snap07_child() {
    if child_mode().as_deref() != Some("snap07") {
        return;
    }
    let bytes = objstm_bomb_pdf();
    let len = bytes.len();
    let (r, peak) = process_peak(|| snapshot_from_bytes(Path::new("bomb.pdf"), bytes, None));
    println!(
        "FILE={len}\nPEAK={peak}\nTOO_COMPLEX={}",
        u8::from(code(&r) == "FILE_TOO_COMPLEX")
    );
}

#[test]
fn snap08_xref_stream_bomb_is_file_too_complex() {
    let d = Doc::new(&[&[HELLO]]);
    let bytes = d.build_with(&XrefStyle::Stream {
        in_objstm: vec![],
        bomb_mib: Some(1024),
    });
    let (r, peak) = thread_peak(|| from_bytes(&bytes));
    assert_eq!(code(&r), "FILE_TOO_COMPLEX", "SNAP-08");
    assert!(
        peak < bytes.len() + (8 << 20),
        "SNAP-08 preflight peak {peak}"
    );
    let fine = d.build_with(&XrefStyle::Stream {
        in_objstm: vec![],
        bomb_mib: None,
    });
    assert_eq!(
        ok(from_bytes(&fine), "SNAP-08 plain xref stream")
            .pages
            .len(),
        1
    );
}

fn answer_42(id: (u32, u16), obj: &mut Object) -> Option<((u32, u16), Object)> {
    if let Object::Dictionary(d) = obj {
        d.set("Mark", 1);
    }
    Some((id, Object::Integer(42)))
}

#[test]
fn snap09_objstm_guard_preserves_objects() {
    let d = Doc::new(&[&[HELLO], &[b"q Q"]]);
    let members = vec![d.pages, 3, d.page_ids[1]];
    let bytes = d.build_with(&XrefStyle::Stream {
        in_objstm: members.clone(),
        bomb_mib: None,
    });
    let ours = ok(from_bytes(&bytes), "SNAP-09").doc;
    let plain = lopdf::Document::load_mem(&bytes).unwrap();
    assert_eq!(
        ours.objects, plain.objects,
        "SNAP-09 guarded load == plain lopdf load"
    );
    assert_eq!(ours.get_pages().len(), 2);
    // Pins lopdf 0.34: top-level return value ignored (the in-place mutation is kept),
    // object-stream members use the returned value.
    let pinned = lopdf::Reader {
        buffer: &bytes,
        document: lopdf::Document::new(),
    }
    .read(Some(answer_42))
    .unwrap();
    match pinned.objects.get(&(d.catalog, 0)) {
        Some(Object::Dictionary(cat)) => assert!(
            cat.has(b"Mark"),
            "SNAP-09 top level keeps the mutated object"
        ),
        other => panic!("SNAP-09 catalog: {other:?}"),
    }
    for m in members {
        assert_eq!(
            pinned.objects.get(&(m, 0)),
            Some(&Object::Integer(42)),
            "SNAP-09 member {m} uses the returned value"
        );
    }
    let Some(engines) = engines_or_skip("snap09_objstm_guard_preserves_objects") else {
        return;
    };
    let s = Scratch::new("snap09");
    let src = s.write("in.pdf", &Doc::new(&[&[HELLO], &[b"q Q"]]).build());
    let out = s.path("objstm.pdf");
    let args = [
        OsString::from("--object-streams=generate"),
        src.into(),
        out.clone().into(),
    ];
    assert_eq!(
        run_tool(&engines.qpdf, &args, false, &RunOpts::default())
            .unwrap()
            .code,
        0
    );
    let generated = std::fs::read(&out).unwrap();
    let ours = ok(read_snapshot(&out), "SNAP-09 qpdf").doc;
    assert!(ours
        .objects
        .values()
        .any(|o| matches!(o, Object::Stream(s) if s.dict.type_is(b"ObjStm"))));
    assert_eq!(
        ours.objects,
        lopdf::Document::load_mem(&generated).unwrap().objects,
        "SNAP-09 qpdf --object-streams=generate"
    );
}

#[test]
fn snap10_deep_nesting_is_file_too_complex() {
    let nested = |n: usize| {
        let mut d = Doc::new(&[&[HELLO]]);
        d.b.add(format!("{}{}", "[".repeat(n), "]".repeat(n)));
        d.build()
    };
    assert_eq!(code(&from_bytes(&nested(100))), "OK", "SNAP-10 100 levels");
    assert_eq!(
        code(&from_bytes(&nested(101))),
        "FILE_TOO_COMPLEX",
        "SNAP-10 101 levels"
    );
    let mut d = Doc::new(&[&[HELLO]]);
    d.b.add(format!("({})", "[<<".repeat(200)));
    d.b.add_stream("", "[[[[".repeat(200).as_bytes());
    d.b.add(format!("<{}>", "ab".repeat(10)));
    assert_eq!(
        code(&from_bytes(&d.build())),
        "OK",
        "SNAP-10 strings, streams, hex strings are skipped"
    );
    let dicts = format!("{}{}", "<< /A ".repeat(101), ">> ".repeat(101));
    let mut d = Doc::new(&[&[HELLO]]);
    d.b.add(dicts);
    assert_eq!(
        code(&from_bytes(&d.build())),
        "FILE_TOO_COMPLEX",
        "SNAP-10 dictionaries count too"
    );
}

fn page_map_code(bytes: &[u8], id: &str) -> Option<String> {
    let engines = engines_or_skip(id)?;
    let s = Scratch::new("pagemap");
    let p = s.write("f.pdf", bytes);
    let snap = ok(read_snapshot(&p), id);
    let pages =
        qpdf_page_map(&engines, &p, &RunOpts::default()).unwrap_or_else(|e| panic!("{id}: {e}"));
    Some(match check_page_map(&snap, &pages) {
        Ok(()) => "OK".to_string(),
        Err(e) => e.code,
    })
}

#[test]
fn snap11_hybrid_objects_only_in_xrefstm_need_repair() {
    let d = Doc::new(&[&[HELLO]]);
    let bytes = d.build_with(&XrefStyle::Hybrid {
        in_objstm: vec![d.page_ids[0]],
        objstm_in_classic: false,
    });
    assert_eq!(
        ok(from_bytes(&bytes), "SNAP-11").pages.len(),
        0,
        "lopdf misses the page"
    );
    if let Some(c) = page_map_code(&bytes, "snap11") {
        assert_eq!(c, "PDF_NEEDS_REPAIR", "SNAP-11");
    }
}

#[test]
fn snap12_kid_without_type_needs_repair() {
    let mut d = Doc::new(&[&[HELLO], &[b"q Q"]]);
    let (pages, content) = (d.pages, d.content_ids[1][0]);
    d.b.set(
        d.page_ids[1],
        format!("<< /Parent {pages} 0 R /Contents {content} 0 R >>"),
    );
    let bytes = d.build();
    assert_eq!(
        ok(from_bytes(&bytes), "SNAP-12").pages.len(),
        1,
        "lopdf skips the kid"
    );
    if let Some(c) = page_map_code(&bytes, "snap12") {
        assert_eq!(c, "PDF_NEEDS_REPAIR", "SNAP-12");
    }
}

fn image_streams_pdf(mib_each: usize) -> Vec<u8> {
    let mut d = Doc::new(&[&[b"q 100 0 0 100 0 0 cm /Im0 Do Q"]]);
    let pixels = vec![0u8; mib_each << 20];
    for _ in 0..2 {
        d.b.add_stream("/Type /XObject /Subtype /Image /Width 1024 /Height 1024 /ColorSpace /DeviceGray /BitsPerComponent 8", &pixels);
    }
    d.build()
}

#[test]
fn snap13_guard_never_clones_streams() {
    let out = run_child_test(
        "pdf_engine::text_edit::tests_io::snap::snap13_child",
        "snap13",
    );
    let (file, peak) = (child_value(&out, "FILE"), child_value(&out, "PEAK"));
    assert_eq!(child_value(&out, "PAGES"), 1);
    assert!(
        file > 200 << 20,
        "SNAP-13 fixture has 200 MiB of image streams"
    );
    assert!(
        (peak as f64) <= 1.3 * file as f64,
        "SNAP-13 peak {peak} ≤ 1.3 × {file}"
    );
}

#[test]
#[ignore = "child process of snap13 (run by it)"]
fn snap13_child() {
    if child_mode().as_deref() != Some("snap13") {
        return;
    }
    let bytes = image_streams_pdf(100);
    let len = bytes.len();
    let (r, peak) = process_peak(|| {
        snapshot_from_bytes(Path::new("images.pdf"), bytes, None).map(|s| s.pages.len())
    });
    println!("FILE={len}\nPEAK={peak}\nPAGES={}", r.unwrap_or(0));
}

#[test]
fn snap14_verification_snapshot_skips_policy_keeps_bounds() {
    let s = Scratch::new("snap14");
    let vcode = |p: &Path, cap: u64| match read_verification_snapshot(p, cap) {
        Ok(_) => "OK".to_string(),
        Err(e) => format!("{}|{}", e.code, e.details.unwrap_or_default()),
    };
    let signed = with_catalog("/Perms << /DocMDP << >> >>", |_| String::new());
    let xfa = with_catalog("/NeedsRendering true", |_| String::new());
    assert_eq!(
        vcode(&s.write("signed.pdf", &signed), 1 << 30),
        "OK",
        "SNAP-14 signed output opens"
    );
    assert_eq!(
        vcode(&s.write("xfa.pdf", &xfa), 1 << 30),
        "OK",
        "SNAP-14 XFA output opens"
    );
    let bomb = vcode(&s.write("bomb.pdf", &objstm_bomb_pdf()), 1 << 30);
    assert!(
        bomb.starts_with("EDIT_VERIFY_FAILED|") && bomb.contains("too large to verify"),
        "SNAP-14 ObjStm bound: {bomb}"
    );
    let mut deep = Doc::new(&[&[HELLO]]);
    deep.b
        .add(format!("{}{}", "[".repeat(101), "]".repeat(101)));
    assert!(
        vcode(&s.write("deep.pdf", &deep.build()), 1 << 30).contains("FILE_TOO_COMPLEX"),
        "SNAP-14 nesting bound"
    );
    let small = s.write("small.pdf", &simple_pdf(HELLO));
    let len = std::fs::metadata(&small).unwrap().len();
    assert_eq!(vcode(&small, len), "OK", "SNAP-14 cap inclusive");
    let over = vcode(&small, len - 1);
    assert!(
        over.starts_with("EDIT_VERIFY_FAILED|")
            && over.contains("too large to verify: FILE_TOO_LARGE"),
        "SNAP-14 cap: {over}"
    );
    let d = Doc::new(&[&[HELLO]]);
    let enc =
        d.b.build(&format!("{} /Encrypt << /Filter /Standard >>", d.trailer()));
    let enc = vcode(&s.write("enc.pdf", &enc), 1 << 30);
    assert!(
        enc.starts_with("EDIT_VERIFY_FAILED|")
            && enc.contains("could not read the checked file: ENCRYPTED"),
        "SNAP-14 encrypted: {enc}"
    );
}

#[test]
fn snap15_stat_matches_detects_same_length_rewrite() {
    let s = Scratch::new("snap15");
    let p = s.write("a.pdf", &simple_pdf(HELLO));
    let snap = ok(read_snapshot(&p), "SNAP-15");
    assert!(stat_matches(&snap), "SNAP-15 unchanged");
    let mut other = simple_pdf(HELLO);
    let pos = other.windows(5).position(|w| w == b"Hello").unwrap();
    other[pos] = b'J';
    std::fs::write(&p, &other).unwrap();
    let f = std::fs::File::options().write(true).open(&p).unwrap();
    f.set_modified(snap.modified.unwrap()).unwrap();
    drop(f);
    assert_eq!(
        std::fs::metadata(&p).unwrap().modified().ok(),
        snap.modified,
        "mtime restored"
    );
    assert!(
        !stat_matches(&snap),
        "SNAP-15 same length + same mtime, different bytes"
    );
    std::fs::write(&p, b"short").unwrap();
    assert!(!stat_matches(&snap));
    std::fs::remove_file(&p).unwrap();
    assert!(!stat_matches(&snap));
}

#[test]
fn snap16_word_style_hybrid_loads_and_agrees() {
    let d = Doc::new(&[&[HELLO], &[b"q", b"Q"]]);
    let bytes = d.build_with(&XrefStyle::Hybrid {
        in_objstm: vec![d.page_ids[0], d.pages],
        objstm_in_classic: true,
    });
    let snap = ok(from_bytes(&bytes), "SNAP-16");
    assert_eq!(
        snap.pages.len(),
        2,
        "SNAP-16 members reached through the classic-listed object stream"
    );
    if let Some(c) = page_map_code(&bytes, "snap16") {
        assert_eq!(c, "OK", "SNAP-16");
    }
    // A /Contents disagreement is found and named.
    let Some(engines) = engines_or_skip("snap16_word_style_hybrid_loads_and_agrees") else {
        return;
    };
    let s = Scratch::new("snap16");
    let p = s.write("two.pdf", &d.build());
    let snap = ok(read_snapshot(&p), "SNAP-16");
    let mut pages = qpdf_page_map(&engines, &p, &RunOpts::default()).unwrap();
    assert!(check_page_map(&snap, &pages).is_ok());
    pages[1].contents.reverse();
    let e = check_page_map(&snap, &pages).unwrap_err();
    assert_eq!(e.code, "PDF_NEEDS_REPAIR");
    assert!(
        e.details.unwrap_or_default().contains("page 2: /Contents"),
        "first difference named"
    );
}
