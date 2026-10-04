//! T4 independent-engine tests (SPEC §E.6, qpdf + Poppler required): IND-01…08 and the B13
//! regression. Poppler — not our width model — reads and renders the page before and after the
//! edit (Phase A check A5). IND-02/03/04 reach A5 by skipping earlier checks through the
//! `#[cfg(test)]` seams only, and also show which earlier check catches the same tampering.

use crate::pdf_engine::text_edit::context::SnapshotContext;
use crate::pdf_engine::text_edit::engines::RunOpts;
use crate::pdf_engine::text_edit::poppler::{
    pdftotext_words, render_dpi, render_page, user_rect_to_pixels, user_rect_to_text_frame,
};
use crate::pdf_engine::text_edit::reasons::TextWarningCode;
use crate::pdf_engine::text_edit::rewrite::SourceTextStyleIn;
use crate::pdf_engine::text_edit::runs::build_page_model;
use crate::pdf_engine::text_edit::snapshot::snapshot_from_bytes;
use crate::pdf_engine::text_edit::testkit::fakes::{self, json_dict};
use crate::pdf_engine::text_edit::testkit::fonts::tounicode_bfchar;
use crate::pdf_engine::text_edit::testkit::producers::{
    self as fx, helvetica_doc, helvetica_page, word_font, DocBuilder, PageSpec,
};
use crate::pdf_engine::text_edit::testkit::{engines_or_skip, Scratch};
use crate::pdf_engine::text_edit::tests_gate::{ed, fails_at, skipping, Ed, Honest};
use lopdf::Object;
use serde_json::json;

/// IND-01: an honest edit per font class passes every Phase A check, A5 included.
#[test]
fn ind_01_honest_edits_on_every_font_class_pass_words_and_pixels() {
    let cases: Vec<(&str, Vec<u8>, &str, &str)> = vec![
        (
            "std14",
            helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET"),
            "Hello",
            "Help",
        ),
        ("std14-times", fx::std14(), "Times line", "Times lines"),
        ("truetype-word", fx::word(), "Due", "Due 7"),
        (
            "type0-sibling",
            fx::word_tr(),
            "Sağlık Bakanlığı Raporu",
            "Sağlığı Bakanlık Raporu",
        ),
        ("symbolic-truetype", fx::libre(), "Libre text", "Libre tex"),
        ("cff", fx::libre_cff(), "Hello Hello", "Hello Hole"),
        ("type0-skia", fx::skia(), "Chrome", "Chrom"),
        ("quartz", fx::quartz(), "Quartz", "Quart"),
        ("type1", fx::pdftex(), "Hello World", "Hello Wet World"),
        ("cid-cff", fx::xetex(), "XeTeX", "TeX"),
        (
            "type1c-tracking",
            fx::indd(),
            "visible layer",
            "visible layers",
        ),
        ("not-embedded", fx::nonemb(), "Not embedded", "Not embed"),
        ("mac-roman", fx::mac_roman(), "caf\u{e9}", "cafe"),
    ];
    for (name, pdf, old, new) in cases {
        let test = format!("ind_01_{name}");
        let Some(h) = Honest::new(&test, pdf, 0, &[ed(old, new)]) else {
            return;
        };
        assert!(
            h.report
                .warnings
                .iter()
                .all(|w| w.code != TextWarningCode::EditNotVisible),
            "IND-01 {name}: the edit is visible"
        );
    }
}

/// A page with two lines in a Word subset: the edited one and an unedited follower line.
fn word_two_lines() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f1 = word_font(&mut d.b, "ABCDEF+Calibri", "Invoice 20267 Due");
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 700 Td (Invoice 2026) Tj ET BT /F1 12 Tf 72 650 Td (Due 2026) Tj ET",
        &format!("/Font << /F1 {f1} 0 R >>"),
    ));
    d.build()
}

/// The output with every `/Widths` entry of page 1's `/F1` scaled by `factor`.
fn widths_scaled(h: &Honest, factor: f64, name: &str) -> std::path::PathBuf {
    let doc = fakes::load(&h.staged);
    let page = fakes::page_ids(&doc)[0];
    let font = doc
        .get_dictionary(page)
        .and_then(|d| d.get(b"Resources"))
        .and_then(Object::as_dict)
        .and_then(|r| r.get(b"Font"))
        .and_then(Object::as_dict)
        .and_then(|f| f.get(b"F1"))
        .and_then(Object::as_reference)
        .expect("/F1");
    let mut d = json_dict(doc.get_dictionary(font).expect("font"));
    if let Some(serde_json::Value::Array(w)) = d.get_mut("/Widths") {
        for v in w.iter_mut() {
            *v = json!(v.as_f64().unwrap_or(0.0) * factor);
        }
    }
    let out = h.path(name);
    fakes::set_value(h.qpdf(), &h.staged, &out, font, d).unwrap_or_else(|e| panic!("{e}"));
    out
}

#[test]
fn ind_02_widths_scaled_in_the_output_fail_the_words() {
    let Some(h) = Honest::new(
        "ind_02",
        word_two_lines(),
        0,
        &[ed("Invoice 2026", "Invoice 2027")],
    ) else {
        return;
    };
    let out = widths_scaled(&h, 1.01, "widths.pdf");
    // Our own re-walk sees the changed font too (A4) …
    let r = skipping(&["A2", "A3"], || h.phase_a(&out));
    fails_at(r, "STATE_CHANGED", "A4", "field=font", "IND-02 A4");
    // … and Poppler's word boxes move: the follower line no longer matches (A5).
    let r = skipping(&["A2", "A3", "A4"], || h.phase_a(&out));
    fails_at(r, "EDIT_VERIFY_FAILED", "A5", "check=words", "IND-02");
}

#[test]
fn b13_independent_engines_judge_the_output_not_our_width_model() {
    // Helvetica with explicit /Widths; the output's widths are 3 % off. With every check that
    // shares our model skipped, Poppler alone still refuses the file.
    let mut widths = String::new();
    for c in 32..=126u8 {
        let w = match c {
            b' ' => 278,
            b'H' => 722,
            b'e' | b'o' | b'p' => 556,
            b'l' => 222,
            _ => 500,
        };
        widths.push_str(&format!("{w} "));
    }
    let mut d = DocBuilder::new();
    let f = d.add(format!(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /FirstChar 32 /LastChar 126 /Widths [{widths}] >>"
    ));
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET BT /F1 12 Tf 72 650 Td (Hello people) Tj ET",
        &format!("/Font << /F1 {f} 0 R >>"),
    ));
    let Some(h) = Honest::new("b13", d.build(), 0, &[ed("Hello", "Help")]) else {
        return;
    };
    let out = widths_scaled(&h, 1.03, "widths.pdf");
    let r = skipping(&["A2", "A3", "A4"], || h.phase_a(&out));
    fails_at(r, "EDIT_VERIFY_FAILED", "A5", "check=words", "B13");
}

#[test]
fn ind_03_old_text_left_extractable_through_actualtext() {
    let pdf = helvetica_page(
        b"BT /F1 12 Tf 72 700 Td (Invoice 2026) Tj ET BT /F1 12 Tf 300 700 Td (tail) Tj ET \
          BT /F1 12 Tf 72 650 Td (Next line) Tj ET",
    );
    let Some(h) = Honest::new("ind_03", pdf, 0, &[ed("Invoice 2026", "Invoice 2027")]) else {
        return;
    };
    let honest = String::from_utf8_lossy(&h.plan.expected_parts[0]).into_owned();
    let tail = "BT /F1 12 Tf 300 700 Td (tail) Tj ET";
    let bad = honest.replace(
        tail,
        &format!("/Span <</ActualText (tail Invoice 2026)>> BDC {tail} EMC"),
    );
    let out = h.path("actualtext.pdf");
    fakes::replace_streams(
        h.qpdf(),
        &h.source,
        &out,
        &[(h.part_id(0), bad.into_bytes())],
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let r = skipping(&["A2", "A3"], || h.phase_a(&out));
    fails_at(r, "EDIT_VERIFY_FAILED", "A4", "", "IND-03 A4");
    let r = skipping(&["A2", "A3", "A4"], || h.phase_a(&out));
    fails_at(r, "EDIT_VERIFY_FAILED", "A5", "still extracted", "IND-03");
}

#[test]
fn ind_04_colour_leak_onto_a_path_fails_the_pixels() {
    let pdf = helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET 300 300 80 80 re f");
    let edits = [Ed {
        old: "Hello",
        new: "Hello",
        style: SourceTextStyleIn {
            fill: Some("#00ff00".into()),
            ..Default::default()
        },
    }];
    let Some(h) = Honest::new("ind_04", pdf, 0, &edits) else {
        return;
    };
    let (plan, digest, out) = h.bad_plan("leak.pdf", |s| s.replace("] TJ 0 g", "] TJ"));
    let r = skipping(&["A4"], || h.phase_a_of(&plan, &digest, &out));
    fails_at(r, "EDIT_VERIFY_FAILED", "A5", "check=render", "IND-04");
}

#[test]
fn ind_05_edit_under_an_opaque_box_is_a_warning() {
    let pdf = helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET 1 g 60 690 200 30 re f");
    let Some(h) = Honest::new("ind_05", pdf, 0, &[ed("Hello", "Help")]) else {
        return;
    };
    assert!(
        h.report
            .warnings
            .iter()
            .any(|w| w.code == TextWarningCode::EditNotVisible),
        "IND-05 EDIT_NOT_VISIBLE: {:?}",
        h.report.warnings
    );
}

#[test]
fn ind_06_frames_on_rotated_and_cropped_pages() {
    let Some(engines) = engines_or_skip("ind_06") else {
        return;
    };
    let dir = Scratch::new("ind_06");
    let opts = RunOpts::default();
    let pages: Vec<(String, Vec<u8>)> = [0, 90, 180, 270]
        .iter()
        .map(|r| {
            (
                format!("rotate {r}"),
                helvetica_doc(
                    b"BT /F1 20 Tf 100 200 Td (X) Tj ET",
                    "",
                    &format!("/Rotate {r}"),
                ),
            )
        })
        .chain(std::iter::once((
            "offset CropBox".to_string(),
            helvetica_doc(
                b"BT /F1 20 Tf 100 200 Td (X) Tj ET",
                "",
                "/CropBox [50 60 500 700]",
            ),
        )))
        .collect();
    for (name, pdf) in pages {
        let path = dir.write("page.pdf", &pdf);
        let snap = snapshot_from_bytes(&path, pdf, None).expect("snapshot");
        let ctx = SnapshotContext::new(snap);
        let m = build_page_model(&ctx, 0, None).expect("model");
        let g = &m.walk.records[0].glyphs[0];
        let geom = &m.walk.geometry;
        // pdftotext: the word box lies in the computed MediaBox frame box.
        let frame = user_rect_to_text_frame(g.bbox, geom);
        let words = pdftotext_words(&engines, &path, 1, &opts).expect("words");
        let w = words.first().expect("one word");
        let (cx, cy) = ((w.x0 + w.x1) / 2.0, (w.y0 + w.y1) / 2.0);
        assert!(
            cx > frame[0] - 2.0
                && cx < frame[2] + 2.0
                && cy > frame[1] - 2.0
                && cy < frame[3] + 2.0,
            "IND-06 {name}: word {w:?} outside the frame box {frame:?}"
        );
        // pdftoppm (default MediaBox render): every dark pixel lies in the computed pixel box.
        let dpi = render_dpi(geom).expect("a render resolution");
        let raster = render_page(&engines, &path, 1, dpi, dir.dir(), &opts).expect("render");
        let b = user_rect_to_pixels(g.bbox, geom, dpi);
        let (mut inside, mut outside) = (0usize, 0usize);
        for y in 0..raster.h {
            for x in 0..raster.w {
                let i = ((y * raster.w + x) * 3) as usize;
                if raster.rgb[i..i + 3].iter().any(|c| *c < 128) {
                    let (xi, yi) = (i64::from(x), i64::from(y));
                    if xi >= b[0] - 2 && xi < b[2] + 2 && yi >= b[1] - 2 && yi < b[3] + 2 {
                        inside += 1;
                    } else {
                        outside += 1;
                    }
                }
            }
        }
        assert!(
            inside > 0 && outside == 0,
            "IND-06 {name}: {inside} inside, {outside} outside {b:?}"
        );
    }
}

#[test]
fn ind_07_anti_aliasing_next_to_dense_text_stays_within_tolerance() {
    let mut content = String::new();
    for i in 0..30 {
        let y = 700.0 - 8.5 * f64::from(i);
        content.push_str(&format!(
            "BT /F1 8 Tf 72 {y} Td (Line {i:02} with dense neighbouring text WWWWWW) Tj ET "
        ));
    }
    let pdf = helvetica_page(content.as_bytes());
    let old = "Line 15 with dense neighbouring text WWWWWW";
    let Some(h) = Honest::new(
        "ind_07",
        pdf,
        0,
        &[ed(old, "Line 15 with denser text MMMM")],
    ) else {
        return;
    };
    assert!(h.report.proofs.len() == 1, "IND-07 passes A5");
}

/// LiberationSans-Italic (in the repo, LICENSE_LIBERATION) as a Type0 Identity-H CIDFontType2
/// whose CIDs are the program's GIDs, for `chars`, with the program's real `/FontBBox`.
fn liberation_italic(
    d: &mut DocBuilder,
    chars: &str,
) -> (u32, std::collections::HashMap<char, u16>) {
    liberation(d, "LiberationSans-Italic", chars)
}

/// A Liberation Sans face from the repo (`file` without `.ttf`) as a Type0 Identity-H
/// CIDFontType2 whose CIDs are the program's GIDs, for `chars`, with the program's real
/// `/FontBBox` (Liberation Sans is metric-compatible with Arial; its box reaches ~2 em right).
pub(crate) fn liberation(
    d: &mut DocBuilder,
    file: &str,
    chars: &str,
) -> (u32, std::collections::HashMap<char, u16>) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../public/pdfjs/standard_fonts/{file}.ttf"));
    let program = std::fs::read(path).expect("a Liberation Sans program");
    let italic = file.contains("Italic");
    let (flags, angle) = if italic { (96, -12) } else { (32, 0) };
    let face = ttf_parser::Face::parse(&program, 0).expect("parse");
    let upem = f64::from(face.units_per_em());
    let em = |v: i16| (f64::from(v) * 1000.0 / upem).round();
    let bbox = face.global_bounding_box();
    let mut gids = std::collections::HashMap::new();
    let (mut w, mut map) = (String::new(), Vec::new());
    for ch in chars.chars() {
        let gid = face.glyph_index(ch).expect("glyph").0;
        let adv = f64::from(
            face.glyph_hor_advance(ttf_parser::GlyphId(gid))
                .unwrap_or(0),
        ) * 1000.0
            / upem;
        w.push_str(&format!("{gid} [{adv:.0}] "));
        map.push((u32::from(gid), 2, ch.to_string()));
        gids.insert(ch, gid);
    }
    let r: Vec<(u32, usize, &str)> = map.iter().map(|(c, l, t)| (*c, *l, t.as_str())).collect();
    let program_id = d.b.add_flate("", &program);
    let descriptor = d.add(format!(
        "<< /Type /FontDescriptor /FontName /ABCDEF+{name} /Flags {flags} \
         /FontBBox [{} {} {} {}] /ItalicAngle {angle} /Ascent 905 /Descent -212 /CapHeight 729 \
         /StemV 80 /FontFile2 {program_id} 0 R >>",
        em(bbox.x_min),
        em(bbox.y_min),
        em(bbox.x_max),
        em(bbox.y_max),
        name = file,
        program_id = program_id,
    ));
    let cid = d.add(format!(
        "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /ABCDEF+{file} \
         /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
         /FontDescriptor {descriptor} 0 R /W [{w}] /CIDToGIDMap /Identity >>"
    ));
    let tounicode = d.b.add_flate("", &tounicode_bfchar(&r));
    let font = d.add(format!(
        "<< /Type /Font /Subtype /Type0 /BaseFont /ABCDEF+{file} /Encoding /Identity-H \
         /DescendantFonts [{cid} 0 R] /ToUnicode {tounicode} 0 R >>"
    ));
    (font, gids)
}

#[test]
fn ind_08_accented_capitals_in_an_italic_face_pass_the_render_check() {
    let mut d = DocBuilder::new();
    let (font, gids) = liberation_italic(&mut d, "ABCİĞŞ");
    let hex: String = "ABC".chars().map(|c| format!("{:04X}", gids[&c])).collect();
    d.page(PageSpec::new(
        format!("BT /F1 36 Tf 72 600 Td <{hex}> Tj ET BT /F1 36 Tf 72 500 Td <{hex}> Tj ET")
            .as_bytes(),
        &format!("/Font << /F1 {font} 0 R >>"),
    ));
    let pdf = d.build();
    let model_text = {
        let snap = snapshot_from_bytes(std::path::Path::new("ind08.pdf"), pdf.clone(), None)
            .expect("snapshot");
        let m = build_page_model(&SnapshotContext::new(snap), 0, None).expect("model");
        m.runs
            .iter()
            .map(|r| (r.text.clone(), r.reason))
            .collect::<Vec<_>>()
    };
    assert!(
        model_text.iter().any(|(t, r)| t == "ABC" && r.is_none()),
        "IND-08 editable: {model_text:?}"
    );
    let Some(h) = Honest::new("ind_08", pdf, 0, &[ed("ABC", "İĞŞ")]) else {
        return;
    };
    assert!(
        h.report.proofs.len() == 1,
        "IND-08 G-RENDER passes with glyph masks"
    );
}

/// "Hello" and a follower "42" drawn right after it on the same line, pen-chained (`Tj` then a
/// `TJ` that starts with a −500 kern), in Helvetica with `/Widths` and, when given, a
/// `/FontBBox` (not embedded: the masks use the fallback box).
pub(crate) fn follower_doc(font_bbox: Option<&str>) -> Vec<u8> {
    let mut widths = String::new();
    for c in 32..=126u8 {
        let w = match c {
            b' ' => 278,
            b'H' => 722,
            b'e' | b'o' | b'p' | b'4' | b'2' => 556,
            b'l' => 222,
            _ => 500,
        };
        widths.push_str(&format!("{w} "));
    }
    let mut d = DocBuilder::new();
    let descriptor = font_bbox.map(|bb| {
        let id = d.add(format!(
            "<< /Type /FontDescriptor /FontName /Helvetica /Flags 32 /FontBBox [{bb}] \
             /ItalicAngle 0 /Ascent 905 /Descent -212 /CapHeight 716 /StemV 80 >>"
        ));
        format!("/FontDescriptor {id} 0 R")
    });
    let f = d.add(format!(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding \
         /FirstChar 32 /LastChar 126 /Widths [{widths}] {} >>",
        descriptor.unwrap_or_default()
    ));
    d.page(PageSpec::new(
        b"BT /F1 12 Tf 72 700 Td (Hello) Tj [-500 (42)] TJ ET",
        &format!("/Font << /F1 {f} 0 R >>"),
    ));
    d.build()
}

/// The same line in embedded Liberation Sans (program glyph boxes, a 2 em wide `/FontBBox`).
pub(crate) fn follower_embedded() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let (font, gids) = liberation(&mut d, "LiberationSans-Regular", "Helop42");
    let hex = |t: &str| -> String { t.chars().map(|c| format!("{:04X}", gids[&c])).collect() };
    d.page(PageSpec::new(
        format!(
            "BT /F1 12 Tf 72 700 Td <{}> Tj [-500 <{}>] TJ ET",
            hex("Hello"),
            hex("42")
        )
        .as_bytes(),
        &format!("/Font << /F1 {font} 0 R >>"),
    ));
    d.build()
}

/// IND-09 (review H1): a compensation 40/1000 em too large moves the same-line follower "42"
/// 0.48 pt right. Our re-walk sees it (A4, `PEN_DRIFT`); with A4 skipped — a width-model error
/// both walks would share, B13 — Poppler's pixels alone must refuse it, whatever the font's
/// `/FontBBox`: the masks follow each glyph (program box or advance-bounded fallback), not the
/// font-wide box that reaches 2 em past every glyph.
#[test]
fn ind_09_same_line_follower_moved_by_a_shared_width_error_fails_the_pixels() {
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("arial-bbox", follower_doc(Some("-665 -325 2000 1006"))),
        ("tight-bbox", follower_doc(Some("-166 -225 1000 931"))),
        ("no-bbox", follower_doc(None)),
        ("embedded", follower_embedded()),
    ];
    for (name, pdf) in cases {
        let test = format!("ind_09_{name}");
        let Some(h) = Honest::new(&test, pdf, 0, &[ed("Hello", "Help")]) else {
            return;
        };
        let honest = String::from_utf8_lossy(&h.plan.runs[0].splices[0].bytes).into_owned();
        assert!(
            honest.ends_with(" -222] TJ"),
            "IND-09 {name}: the honest compensation keeps the follower: {honest}"
        );
        let (plan, digest, out) = h.bad_plan(&format!("{name}.pdf"), |s| {
            s.replace(" -222] TJ", " -262] TJ")
        });
        fails_at(
            h.phase_a_of(&plan, &digest, &out),
            "PEN_DRIFT",
            "A4",
            "drift",
            &format!("IND-09 {name} A4"),
        );
        let r = skipping(&["A4"], || h.phase_a_of(&plan, &digest, &out));
        fails_at(
            r,
            "EDIT_VERIFY_FAILED",
            "A5",
            "check=render",
            &format!("IND-09 {name}"),
        );
    }
}

/// The glyphs of the IND-10 font: (char, advance, outline box) in thousandths of an em. "j" reaches
/// left of its origin and below the baseline (negative `x_min` and `y_min`).
const BOX_GLYPHS: [(char, i32, [i32; 4]); 8] = [
    ('H', 700, [50, 0, 650, 700]),
    ('e', 550, [40, -10, 510, 520]),
    ('l', 250, [60, 0, 190, 720]),
    ('o', 600, [40, -10, 560, 520]),
    ('p', 600, [50, -200, 560, 520]),
    ('j', 300, [-100, -200, 250, 700]),
    ('4', 550, [30, 0, 520, 700]),
    ('2', 550, [40, 0, 510, 700]),
];

/// "Hello" and the same-line follower "42" (as in `follower_doc`) in a Type0 font whose CID-keyed
/// CFF program (CID = GID = position in `BOX_GLYPHS` + 1) draws box glyphs with `units` font
/// units per em, and carries `top` / `fd` as its Top DICT / Font DICT `FontMatrix` operands; as a
/// bare `CIDFontType0C` program or, with `opentype`, the `CFF ` table of an `OTTO` font (1,000
/// units per em). Poppler (FreeType) combines the two matrices; ttf-parser reads the Top one only.
fn cid_cff_follower(top: Option<&[u8]>, fd: Option<&[u8]>, units: i32, opentype: bool) -> Vec<u8> {
    use crate::pdf_engine::text_edit::testkit::cff::{t2, CffBuilder};
    use crate::pdf_engine::text_edit::testkit::fonts::{add_type0, Program, Type0Font};
    use crate::pdf_engine::text_edit::testkit::ttf::TtfBuilder;
    let scale = |v: i32| v * units / 1000;
    let mut cff = CffBuilder::new("ABCDEF+BoxCID");
    for (i, (_, _, [x0, y0, x1, y1])) in BOX_GLYPHS.iter().enumerate() {
        let (x0, y0, x1, y1) = (scale(*x0), scale(*y0), scale(*x1), scale(*y1));
        let mut cs: Vec<u8> = [x0, y0].iter().flat_map(|v| t2(*v)).collect();
        cs.push(21); // rmoveto
        for v in [x1 - x0, 0, 0, y1 - y0, x0 - x1, 0] {
            cs.extend(t2(v));
        }
        cs.extend([5, 14]); // rlineto endchar
        cff = cff.raw_cid_glyph(i as u16 + 1, cs);
    }
    if let Some(t) = top {
        cff.top_padding = [t, &[12, 7][..]].concat();
    }
    let program = cff.build();
    let program = fd.map_or(program.clone(), |m| {
        fakes::cff_with_font_dict_matrix(&program, m)
    });
    let mut f = Type0Font::new("CIDFontType0", "ABCDEF+BoxCID");
    f.program = if opentype {
        let mut t = TtfBuilder::new();
        t.cff = Some(program);
        for (i, (_, advance, _)) in BOX_GLYPHS.iter().enumerate() {
            t.glyph(&format!("cid{}", i + 1), true, *advance as u16);
        }
        Program::OpenType(t.build())
    } else {
        Program::CidCff(program)
    };
    let widths: Vec<String> = BOX_GLYPHS.iter().map(|g| g.1.to_string()).collect();
    f.w = Some(format!("[1 [{}]]", widths.join(" ")));
    let map: Vec<(u32, usize, String)> = BOX_GLYPHS
        .iter()
        .enumerate()
        .map(|(i, g)| (i as u32 + 1, 2, g.0.to_string()))
        .collect();
    let r: Vec<(u32, usize, &str)> = map.iter().map(|(c, l, t)| (*c, *l, t.as_str())).collect();
    f.tounicode = Some(tounicode_bfchar(&r));
    let hex = |t: &str| -> String {
        t.chars()
            .map(|c| {
                let i = BOX_GLYPHS
                    .iter()
                    .position(|g| g.0 == c)
                    .expect("a box glyph");
                format!("{:04X}", i + 1)
            })
            .collect()
    };
    let mut d = DocBuilder::new();
    let font = add_type0(&mut d.b, &f);
    d.page(PageSpec::new(
        format!(
            "BT /F1 12 Tf 72 700 Td <{}> Tj [-500 <{}>] TJ ET",
            hex("Hello"),
            hex("42")
        )
        .as_bytes(),
        &format!("/Font << /F1 {font} 0 R >>"),
    ));
    d.build()
}

/// IND-10 (review round 1, H-1): a CID-keyed CFF program whose Font DICT carries the
/// `FontMatrix` — Top `[1 0 0 1 0 0]` with Font DICT `[0.001 …]`, or the default Top with Font DICT
/// `[0.0005 …]` over 2,000 units per em — is drawn at its true size by Poppler, while the Top
/// matrix alone gives a 1,000× or 2× scale (bare, and inside OpenType). Its glyph boxes are not
/// taken from the program (no clamped 5 em box around the "j"): the honest edit passes A5, and a
/// 0.48 pt shift of the same-line follower fails A5 with A4 skipped. The plain programs (controls)
/// use their own boxes.
#[test]
fn ind_10_cid_cff_font_dict_matrices_do_not_widen_the_masks() {
    const IDENTITY: [u8; 6] = [140, 139, 139, 140, 139, 139];
    const MILLI: [u8; 12] = [
        30, 0x0a, 0x00, 0x1f, 139, 139, 30, 0x0a, 0x00, 0x1f, 139, 139,
    ];
    const HALF_MILLI: [u8; 14] = [
        30, 0x0a, 0x00, 0x05, 0xff, 139, 139, 30, 0x0a, 0x00, 0x05, 0xff, 139, 139,
    ];
    let cases: Vec<(&str, Vec<u8>, f64)> = vec![
        ("plain", cid_cff_follower(None, None, 1000, false), 0.35),
        (
            "top-identity-fd-milli",
            cid_cff_follower(Some(&IDENTITY), Some(&MILLI), 1000, false),
            0.6,
        ),
        (
            "fd-half-milli",
            cid_cff_follower(None, Some(&HALF_MILLI), 2000, false),
            0.6,
        ),
        (
            "opentype-plain",
            cid_cff_follower(None, None, 1000, true),
            0.35,
        ),
        (
            "opentype-fd-half-milli",
            cid_cff_follower(None, Some(&HALF_MILLI), 2000, true),
            0.6,
        ),
    ];
    for (name, pdf, j_width_em) in cases {
        let test = format!("ind_10_{name}");
        let Some(h) = Honest::new(&test, pdf, 0, &[ed("Hello", "Hellj")]) else {
            return;
        };
        // The new "j" (fifth glyph): its program box (0.35 em wide), or the fallback box (advance
        // 0.3 em + 0.1 em left + 0.2 em right, 1.3 em tall), never a box scaled by the Top matrix.
        let j = h.plan.runs[0].expected.mask_boxes[4];
        let (w, ht) = ((j[2] - j[0]) / 12.0, (j[3] - j[1]) / 12.0);
        assert!(
            (w - j_width_em).abs() < 1e-6 && ht <= 1.3 + 1e-6,
            "IND-10 {name}: mask of \"j\" {w} x {ht} em ({j:?})"
        );
        let honest = String::from_utf8_lossy(&h.plan.runs[0].splices[0].bytes).into_owned();
        assert!(
            honest.ends_with(" -300] TJ"),
            "IND-10 {name}: the honest compensation keeps the follower: {honest}"
        );
        let (plan, digest, out) = h.bad_plan(&format!("{name}.pdf"), |s| {
            s.replace(" -300] TJ", " -340] TJ")
        });
        fails_at(
            h.phase_a_of(&plan, &digest, &out),
            "PEN_DRIFT",
            "A4",
            "drift",
            &format!("IND-10 {name} A4"),
        );
        let r = skipping(&["A4"], || h.phase_a_of(&plan, &digest, &out));
        fails_at(
            r,
            "EDIT_VERIFY_FAILED",
            "A5",
            "check=render",
            &format!("IND-10 {name}"),
        );
    }
}

/// The live check's invoice row (review-T5 live B2): a description, a quantity and a price on one
/// baseline.
fn table_row() -> Vec<u8> {
    helvetica_page(
        b"BT /F1 11 Tf 58 700 Td (Desk lamp) Tj ET BT /F1 11 Tf 132 700 Td (4) Tj ET \
          BT /F1 11 Tf 156 700 Td (120.00) Tj ET BT /F1 11 Tf 58 650 Td (Next row) Tj ET",
    )
}

/// IND-11 (review-T5 live B2): a changed line may run into the next table cells — Poppler then
/// orders the cell words between the new words — and A5 passes; but a word of those cells that
/// no longer reads the same at its place (here re-labelled through ActualText, which changes no
/// pixel) fails A5, even when the edit does not overlap it.
#[test]
fn ind_11_neighbours_on_the_edited_line_are_set_aside_and_must_still_read_the_same() {
    let Some(h) = Honest::new(
        "ind_11",
        table_row(),
        0,
        &[ed("Desk lamp", "Desk lamp with a long cable")],
    ) else {
        return;
    };
    assert_eq!(
        h.report.warnings.len(),
        0,
        "IND-11 the overlap is the planner's warning, not A5's"
    );
    let Some(h) = Honest::new("ind_11b", table_row(), 0, &[ed("Desk lamp", "Desk lamps")]) else {
        return;
    };
    let honest = String::from_utf8_lossy(&h.plan.expected_parts[0]).into_owned();
    let cell = "BT /F1 11 Tf 132 700 Td (4) Tj ET";
    assert!(honest.contains(cell), "IND-11 fixture");
    let bad = honest.replace(cell, &format!("/Span <</ActualText (5)>> BDC {cell} EMC"));
    let out = h.path("relabelled.pdf");
    fakes::replace_streams(
        h.qpdf(),
        &h.source,
        &out,
        &[(h.part_id(0), bad.into_bytes())],
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let r = skipping(&["A2", "A3", "A4"], || h.phase_a(&out));
    fails_at(
        r,
        "EDIT_VERIFY_FAILED",
        "A5",
        "next to the edited line",
        "IND-11",
    );
}

mod followers;
mod overlap;
