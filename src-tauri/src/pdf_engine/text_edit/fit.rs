//! Width helpers shared with the DTOs (`word_space`) and, under `cfg(test)`, the editor's width
//! estimate (`fit/estimate.rs`, MEAS-01 golden): the frontend estimates while typing (SPEC §A.8,
//! §D.4); nothing here is authoritative — every commit is planned and verified by
//! `rewrite`/`verify`.

#[cfg(test)]
mod estimate;

use crate::pdf_engine::text_edit::fonts::FontModel;
#[cfg(test)]
pub use estimate::{estimate_caret_offsets, estimate_delta_pt};

/// Whether the font types a space as the single byte 0x20 (so `Tw` applies to it).
pub fn word_space(model: &FontModel) -> bool {
    model
        .code_for(' ', &[], &[])
        .is_some_and(|c| model.is_word_space(c))
}

#[cfg(test)]
pub fn write_measure_golden(path: &std::path::Path) -> std::io::Result<()> {
    let json = golden::build();
    let mut text = serde_json::to_string_pretty(&json).map_err(std::io::Error::other)?;
    text.push('\n');
    std::fs::write(path, text)
}

/// The golden cases (MEAS-01), in the frontend's DTO shapes (`TextRun`, `TextFont`).
#[cfg(test)]
pub(crate) mod golden {
    use super::{estimate_caret_offsets, estimate_delta_pt, word_space};
    use crate::pdf_engine::text_edit::context::SnapshotContext;
    use crate::pdf_engine::text_edit::fonts::{face_surface, FamilyHint, FontKey, FontModel};
    use crate::pdf_engine::text_edit::reasons::Face;
    use crate::pdf_engine::text_edit::rewrite::SourceTextStyleIn;
    use crate::pdf_engine::text_edit::runs::{build_page_model, PageModel, SpaceMode, TextRun};
    use crate::pdf_engine::text_edit::snapshot::snapshot_from_bytes;
    use crate::pdf_engine::text_edit::testkit::fonts::{
        add_type0, glyph_name, tounicode_bfchar, Program, Type0Font,
    };
    use crate::pdf_engine::text_edit::testkit::producers::{self as fx, DocBuilder, PageSpec};
    use crate::pdf_engine::text_edit::testkit::ttf::TtfBuilder;
    use serde_json::{json, Value};
    use std::sync::Arc;

    pub(crate) fn key(m: &FontModel) -> String {
        match m.key {
            FontKey::Indirect((n, g)) => format!("f{n}-{g}"),
            FontKey::Direct { name_hash, .. } => format!("d{name_hash:x}"),
        }
    }

    fn face_str(f: Face) -> &'static str {
        match f {
            Face::Regular => "regular",
            Face::Bold => "bold",
            Face::Italic => "italic",
            Face::BoldItalic => "boldItalic",
        }
    }

    fn font_dto(m: &FontModel) -> Value {
        let alphabet = m.alphabet();
        json!({
            "key": key(m),
            "displayName": m.display_name,
            "familyHint": match m.family_hint { FamilyHint::Serif => "serif", FamilyHint::Sans => "sans", FamilyHint::Mono => "mono" },
            "embedded": m.embedded,
            "subset": m.subset,
            "alphabet": alphabet.iter().map(|(c, _)| *c).collect::<String>(),
            "widths": alphabet.iter().map(|(_, w)| *w).collect::<Vec<f64>>(),
            "wordSpace": word_space(m),
        })
    }

    fn keys_of(fonts: &[(Vec<u8>, Arc<FontModel>)], names: &[Vec<u8>]) -> Vec<String> {
        names
            .iter()
            .filter_map(|n| fonts.iter().find(|(x, _)| x == n).map(|(_, m)| key(m)))
            .collect()
    }

    fn run_dto(model: &PageModel, run: &TextRun) -> Value {
        let fonts = &model.walk.page_fonts;
        let primary = run
            .surface
            .first()
            .and_then(|n| fonts.iter().find(|(x, _)| x == n))
            .map(|(_, m)| Arc::clone(m));
        let face = |f: Face| {
            if f == run.face {
                return json!({ "available": true, "surface": keys_of(fonts, &run.surface) });
            }
            let s = primary.as_ref().and_then(|p| face_surface(fonts, p, f));
            json!({
                "available": s.is_some(),
                "surface": s.map(|s| s.fonts.iter().map(|(_, m)| key(m)).collect::<Vec<_>>()).unwrap_or_default(),
            })
        };
        json!({
            "id": run.id,
            "order": run.order,
            "line": run.line,
            "text": run.text,
            "rect": { "x": run.rect[0], "y": run.rect[1], "w": run.rect[2], "h": run.rect[3] },
            "origin": { "x": run.origin.0, "y": run.origin.1 },
            "dir": { "x": run.dir.0, "y": run.dir.1 },
            "ascent": run.ascent,
            "descent": run.descent,
            "caretOffsets": run.caret_offsets,
            "editable": true,
            "reason": null,
            "metrics": {
                "surface": keys_of(fonts, &run.surface),
                "tfSize": run.tfs,
                "effectiveSize": run.effective_size,
                "charSpacing": run.tc,
                "wordSpacing": run.tw,
                "hScale": run.th,
                "textToUser": run.text_to_user_x,
                "letterSpacingPt": run.tc * run.th * run.text_to_user_x,
                "spaceMode": match run.space_mode { SpaceMode::Glyph => "glyph", SpaceMode::Kern => "kern" },
                "kernSpace": run.kern_space,
                "originalWidth": run.original_extent,
                "visibleExtent": run.visible_extent,
                "nextObstacle": run.next_obstacle,
            },
            "style": {
                "fill": run.fill_hex,
                "sizeChangeable": !run.font_from_extgstate,
                "colourChangeable": run.tr == 0,
                "face": face_str(run.face),
                "faces": {
                    "regular": face(Face::Regular),
                    "bold": face(Face::Bold),
                    "italic": face(Face::Italic),
                    "boldItalic": face(Face::BoldItalic),
                },
            },
            "substituted": run.substituted,
        })
    }

    fn style_dto(s: &SourceTextStyleIn) -> Value {
        let mut m = serde_json::Map::new();
        if let Some(v) = s.size_pt {
            m.insert("sizePt".into(), json!(v));
        }
        if let Some(f) = s.face {
            m.insert("face".into(), json!(face_str(f)));
        }
        if let Some(f) = &s.fill {
            m.insert("fill".into(), json!(f));
        }
        if let Some(v) = s.letter_spacing_pt {
            m.insert("letterSpacingPt".into(), json!(v));
        }
        Value::Object(m)
    }

    fn model_of(pdf: Vec<u8>) -> PageModel {
        let snap = snapshot_from_bytes(std::path::Path::new("golden.pdf"), pdf, None)
            .unwrap_or_else(|e| panic!("golden fixture: {e}"));
        let ctx = SnapshotContext::new(snap);
        build_page_model(&ctx, 0, None).unwrap_or_else(|e| panic!("golden model: {e}"))
    }

    fn case(
        name: &str,
        pdf: Vec<u8>,
        run_text: &str,
        text: &str,
        style: SourceTextStyleIn,
    ) -> Value {
        let model = model_of(pdf);
        let run = model
            .runs
            .iter()
            .find(|r| r.text == run_text)
            .unwrap_or_else(|| panic!("golden {name}: no run {run_text:?}"));
        let fonts = &model.walk.page_fonts;
        json!({
            "name": name,
            "run": run_dto(&model, run),
            "fonts": fonts.iter().map(|(_, m)| font_dto(m)).collect::<Vec<_>>(),
            "text": text,
            "style": style_dto(&style),
            "deltaPt": estimate_delta_pt(run, fonts, text, &style),
            "caretOffsets": estimate_caret_offsets(run, fonts, text, &style),
        })
    }

    /// Helvetica and Helvetica-Bold on one page (a bold sibling for the face case).
    fn two_faces() -> Vec<u8> {
        let mut d = DocBuilder::new();
        let r = d.add(fx::HELVETICA);
        let b = d.add(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>",
        );
        d.page(PageSpec::new(
            b"BT /F1 12 Tf 72 700 Td (Total due) Tj ET BT /F2 12 Tf 72 680 Td (Bold) Tj ET",
            &format!("/Font << /F1 {r} 0 R /F2 {b} 0 R >>"),
        ));
        d.build()
    }

    /// FX-WORD-TR's shape (a WinAnsi TrueType subset at 500 and its Identity-H companion for
    /// "ğışİ", one `BT … Tm` per font segment, joined into one run) with the companion's widths
    /// at 640: in FX-WORD-TR every width is 500, the missing-glyph width too, so `deltaPt` could
    /// not tell which font a character was measured in.
    fn sibling_widths() -> Vec<u8> {
        const COMPANION_WIDTH: f64 = 640.0;
        let mut d = DocBuilder::new();
        let f1 = fx::word_font(&mut d.b, "ABCDEF+Calibri", "Sa lk Bakan Raporu");
        let tr = fx::WORD_TR_CID_CHARS;
        let mut ttf = TtfBuilder::new();
        let mut map = Vec::new();
        for (i, ch) in tr.chars().enumerate() {
            ttf.unicode_glyph(ch, &glyph_name(ch), true);
            map.push((i as u32 + 1, 2, ch.to_string()));
        }
        let refs: Vec<(u32, usize, &str)> =
            map.iter().map(|(c, l, t)| (*c, *l, t.as_str())).collect();
        let mut companion = Type0Font::new("CIDFontType2", "ABCDEF+Calibri");
        companion.program = Program::TrueType(ttf.build());
        companion.cid_to_gid = Some(None);
        companion.flags = Some(32);
        companion.w = Some(format!(
            "[1 [{}]]",
            vec![COMPANION_WIDTH.to_string(); map.len()].join(" ")
        ));
        companion.tounicode = Some(tounicode_bfchar(&refs));
        let f2 = add_type0(&mut d.b, &companion);
        let segs: [(&str, String, usize, f64); 7] = [
            ("F1", "(Sa)".into(), 2, 500.0),
            (
                "F2",
                format!("<{}>", fx::cid_hex(tr, "ğ")),
                1,
                COMPANION_WIDTH,
            ),
            ("F1", "(l)".into(), 1, 500.0),
            (
                "F2",
                format!("<{}>", fx::cid_hex(tr, "ı")),
                1,
                COMPANION_WIDTH,
            ),
            ("F1", "(k Bakanl)".into(), 8, 500.0),
            (
                "F2",
                format!("<{}>", fx::cid_hex(tr, "ığı")),
                3,
                COMPANION_WIDTH,
            ),
            ("F1", "( Raporu)".into(), 7, 500.0),
        ];
        let mut x = 72.0f64;
        let mut body = String::new();
        for (font, shown, n, width) in segs {
            body.push_str(&format!(
                "BT /{font} 11 Tf 1 0 0 1 {x:.4} 700 Tm [{shown}]TJ ET "
            ));
            x += width / 1000.0 * 11.0 * n as f64;
        }
        d.page(PageSpec::new(
            body.as_bytes(),
            &format!("/Font << /F1 {f1} 0 R /F2 {f2} 0 R >>"),
        ));
        d.build()
    }

    pub(crate) fn build() -> Value {
        let plain = SourceTextStyleIn::default;
        let cases = vec![
            case(
                "std14-helvetica-12",
                fx::helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET"),
                "Hello",
                "Help me",
                plain(),
            ),
            case(
                "tz-80",
                fx::helvetica_page(b"BT /F1 12 Tf 80 Tz 72 700 Td (Hello) Tj ET"),
                "Hello",
                "Hello world",
                plain(),
            ),
            case(
                "tc-0.5",
                fx::helvetica_page(b"BT /F1 12 Tf 0.5 Tc 72 700 Td (Hello) Tj ET"),
                "Hello",
                "Hi",
                plain(),
            ),
            case(
                "kern-space",
                fx::pdftex(),
                "Hello World",
                "Hello Wide World",
                plain(),
            ),
            case(
                "sibling-surface",
                sibling_widths(),
                "Sağlık Bakanlığı Raporu",
                "Sağlık Bakanlığı Raporları",
                plain(),
            ),
            case(
                "size-change",
                fx::helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET"),
                "Hello",
                "Hello",
                SourceTextStyleIn {
                    size_pt: Some(14.0),
                    ..plain()
                },
            ),
            case(
                "letter-spacing",
                fx::helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello) Tj ET"),
                "Hello",
                "Hello",
                SourceTextStyleIn {
                    letter_spacing_pt: Some(0.5),
                    ..plain()
                },
            ),
            case(
                "face-bold",
                two_faces(),
                "Total due",
                "Total due",
                SourceTextStyleIn {
                    face: Some(Face::Bold),
                    ..plain()
                },
            ),
        ];
        json!({
            "version": 1,
            "about": "Rust fit::estimate_delta_pt / estimate_caret_offsets on generated fixtures (MEAS-01). Regenerate: OFFPDF_UPDATE_GOLDEN=1 cargo test --lib -j 6 meas_01",
            "tolerance": 1e-6,
            "cases": cases,
        })
    }
}
