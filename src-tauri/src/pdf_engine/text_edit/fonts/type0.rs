//! Type0 fonts (SPEC §A.2, §B.9): `/Identity-H` with a `CIDFontType2` (`FontFile2`, or
//! `FontFile3 /OpenType`) or `CIDFontType0` (`FontFile3 /CIDFontType0C` or `/OpenType`)
//! descendant. Codes are 2 bytes, CID = code. Text comes from ToUnicode only (§A.3.1 item 4);
//! widths from `/W` (both forms) else `/DW` (default 1000; present but not a number →
//! `FONT_UNSUPPORTED`); glyphs through `/CIDToGIDMap` (CIDFontType2) or the CFF's CID → GID map
//! (CID-keyed) / GID = CID (name-keyed).
//!
//! Only codes that ToUnicode maps (any source length, value ≤ 0xFFFF) get a `CodeInfo` (no other
//! code can be decoded or typed); `FontModel::width` answers every other code from the `/W`
//! table.

use super::encodings::{display_text, usable_text, writable_char};
use super::glyph_budget::WorkMeter;
use super::program::{cff_cid_to_gid, face_metrics, OutlineMemo, Outlines};
use super::simple::{load_tounicode, TuState};
use super::tounicode::Lookup;
use super::{
    is_whitespace_char, number_of, vertical_metrics, Code, CodeInfo, Descriptor, FontClass,
    FontModel, Loader,
};
use crate::pdf_engine::text_edit::limits::{
    CIDTOGID_MAX_BYTES, FONT_PROGRAM_MAX_DECODED, W_ENTRIES_MAX,
};
use crate::pdf_engine::text_edit::reasons::TextReason;
use lopdf::{Dictionary, Object, ObjectId, Stream};
use std::collections::BTreeMap;
use ttf_parser::Face;

/// Largest CID an Identity-H code can name.
const CID_MAX: u32 = 0xFFFF;
const DEFAULT_DW: f64 = 1000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CidKind {
    Type0,
    Type2,
}

enum CidToGid {
    Identity,
    Map(Vec<u8>),
}

pub(super) fn load<'a>(
    loader: &mut Loader<'a, '_>,
    mut model: FontModel,
    dict: &'a Dictionary,
) -> FontModel {
    let type0_base =
        String::from_utf8_lossy(loader.name(dict, b"BaseFont").unwrap_or_default()).into_owned();
    let descendant = match loader.get(dict, b"DescendantFonts") {
        Some(Object::Array(items)) => {
            items
                .first()
                .and_then(|d| loader.resolve(d))
                .and_then(|(_, o)| match o {
                    Object::Dictionary(d) => Some(d),
                    _ => None,
                })
        }
        _ => None,
    };
    // The encoding is checked first so that a vertical font says so whatever else is wrong.
    if let Err(reason) = check_encoding(loader, dict, descendant) {
        if reason == TextReason::Vertical {
            model.vertical = true;
        }
        model.refuse(reason);
    }
    let Some(cid_font) = descendant else {
        model.set_names(&strip_cmap_suffix(&type0_base), &Default::default());
        model.refuse(TextReason::FontUnsupported);
        return model;
    };
    // The descendant's BaseFont is the font's own name (Type0 names may carry "-Identity-H").
    let raw_base = loader
        .name(cid_font, b"BaseFont")
        .map(|n| String::from_utf8_lossy(n).into_owned())
        .unwrap_or_else(|| strip_cmap_suffix(&type0_base));
    let desc = match Descriptor::read(loader, cid_font) {
        Ok(d) => d,
        Err(reason) => {
            model.set_names(&raw_base, &Default::default());
            model.refuse(reason);
            return model;
        }
    };
    model.set_names(&raw_base, &desc.hints());
    let (ascent, descent) = vertical_metrics(&desc, None, None);
    model.ascent = ascent;
    model.descent = descent;
    let kind = match loader.name(cid_font, b"Subtype") {
        Some(b"CIDFontType2") => CidKind::Type2,
        Some(b"CIDFontType0") => CidKind::Type0,
        _ => {
            model.refuse(TextReason::FontUnsupported);
            return model;
        }
    };
    match read_w(loader, cid_font) {
        Ok(widths) => model.cid_widths = widths,
        Err(reason) => model.refuse(reason),
    }
    let default_width = match loader.get(cid_font, b"DW") {
        None => Some(DEFAULT_DW),
        Some(dw) => number_of(dw),
    };
    match default_width {
        Some(dw) => model.default_width = dw,
        None => {
            model.default_width = DEFAULT_DW;
            model.refuse(TextReason::FontUnsupported);
        }
    }
    let cid_to_gid = if kind == CidKind::Type2 {
        match read_cid_to_gid(loader, cid_font) {
            Ok(map) => Some(map),
            Err(reason) => {
                model.refuse(reason);
                None
            }
        }
    } else {
        None
    };
    let tu = load_tounicode(loader, dict);
    match &tu {
        TuState::Absent => model.refuse(TextReason::NoTounicode),
        TuState::Broken => model.refuse(TextReason::AmbiguousUnicode),
        TuState::Ready(_) => {}
    }
    let desc_dict = loader.dict(cid_font, b"FontDescriptor");
    let program = match select_program(loader, desc_dict, kind) {
        Ok(Some(p)) => Some(p),
        Ok(None) => {
            model.refuse(TextReason::FontNotEmbedded);
            None
        }
        Err(reason) => {
            model.embedded = true;
            model.refuse(reason);
            None
        }
    };
    model.class = Some(match kind {
        CidKind::Type2 => FontClass::Type0Cid2,
        CidKind::Type0 => FontClass::Type0Cid0,
    });
    let TuState::Ready(tu) = tu else {
        return model;
    };
    let values: Vec<u32> = tu.codes().filter(|v| *v <= CID_MAX).collect();
    let mut infos: Vec<(u32, Option<String>, f64)> = Vec::with_capacity(values.len());
    for value in values {
        let raw = match tu.lookup(value) {
            Lookup::Text(t) => Some(t.to_string()),
            Lookup::Undecodable | Lookup::Absent => None,
        };
        infos.push((value, raw, model.width(Code { value, len: 2 })));
    }
    let presence = match program {
        Some((id, stream, is_open_type)) => {
            model.embedded = true;
            match loader.stream_bytes(id, stream, FONT_PROGRAM_MAX_DECODED) {
                Ok(bytes) => {
                    let gids = Glyphs::new(&bytes, is_open_type, kind, cid_to_gid.as_ref());
                    match gids {
                        Some(glyphs) => {
                            if let Some(metrics) = glyphs.metrics {
                                let (a, d) = vertical_metrics(&desc, Some(metrics), None);
                                model.ascent = a;
                                model.descent = d;
                            }
                            if let Some(reason) = glyphs.outlines.refusal() {
                                model.refuse(reason);
                            }
                            let mut memo = OutlineMemo::default();
                            let mut meter = loader.work_meter();
                            let found = infos
                                .iter()
                                .map(|(cid, raw, width)| {
                                    let raw = raw.as_deref();
                                    glyphs.presence(&mut memo, &mut meter, *cid, raw, *width)
                                })
                                .collect();
                            loader.settle(&meter);
                            found
                        }
                        None => {
                            model.refuse(TextReason::FontProgramUnreadable);
                            Vec::new()
                        }
                    }
                }
                Err(_) => {
                    model.refuse(TextReason::FontProgramUnreadable);
                    Vec::new()
                }
            }
        }
        None => Vec::new(),
    };
    let mut codes = Vec::with_capacity(infos.len());
    for (i, (cid, raw, width)) in infos.into_iter().enumerate() {
        let (gid, drawable) = presence.get(i).copied().unwrap_or((None, false));
        let text = raw.as_deref().filter(|t| usable_text(t)).map(display_text);
        let single = raw.as_deref().and_then(|t| {
            let mut chars = t.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Some(c),
                _ => None,
            }
        });
        let typeable_as =
            single.filter(|ch| text.is_some() && writable_char(*ch) && drawable && width > 0.0);
        codes.push((
            cid,
            CodeInfo {
                text,
                glyph_name: None,
                gid,
                width1000: Some(width),
                drawable,
                typeable_as,
            },
        ));
    }
    model.codes = codes.into_iter().collect();
    model
}

/// `ABCDEF+Calibri-Identity-H` → `ABCDEF+Calibri`.
fn strip_cmap_suffix(name: &str) -> String {
    for suffix in ["-Identity-H", "-Identity-V"] {
        if let Some(stripped) = name.strip_suffix(suffix) {
            return stripped.to_string();
        }
    }
    name.to_string()
}

/// `/Identity-H` only; `-V` names, `/WMode 1` (of the CIDFont, when there is one, or of an
/// embedded CMap) → `VERTICAL`; other CMaps → `UNSUPPORTED_ENCODING`.
fn check_encoding<'a>(
    loader: &Loader<'a, '_>,
    dict: &'a Dictionary,
    cid_font: Option<&'a Dictionary>,
) -> Result<(), TextReason> {
    let wmode = |d: &'a Dictionary| matches!(loader.get(d, b"WMode"), Some(Object::Integer(1)));
    if cid_font.is_some_and(wmode) {
        return Err(TextReason::Vertical);
    }
    match loader.get(dict, b"Encoding") {
        Some(Object::Name(name)) if name.as_slice() == b"Identity-H" => Ok(()),
        Some(Object::Name(name)) if name.ends_with(b"-V") => Err(TextReason::Vertical),
        Some(Object::Name(_)) => Err(TextReason::UnsupportedEncoding),
        Some(Object::Stream(s)) if wmode(&s.dict) => Err(TextReason::Vertical),
        Some(Object::Stream(_)) => Err(TextReason::UnsupportedEncoding),
        _ => Err(TextReason::FontUnsupported),
    }
}

/// `/W`: `c [w1 w2 …]` and `cfirst clast w`; each span ≤ 65,535 CIDs, at most `W_ENTRIES_MAX`
/// CIDs described in total; later entries win. Returns `(cid, width)` sorted by CID.
fn read_w<'a>(
    loader: &Loader<'a, '_>,
    cid_font: &'a Dictionary,
) -> Result<Vec<(u32, f64)>, TextReason> {
    let present = cid_font.get(b"W").is_ok_and(|w| !matches!(w, Object::Null));
    if !present {
        return Ok(Vec::new());
    }
    let Some(Object::Array(items)) = loader.get(cid_font, b"W") else {
        return Err(TextReason::FontUnsupported);
    };
    let resolved = |o: &'a Object| loader.resolve(o).map(|(_, v)| v);
    let cid = |o: &'a Object| match resolved(o) {
        Some(Object::Integer(v)) => u32::try_from(*v).ok(),
        _ => None,
    };
    let mut map: BTreeMap<u32, f64> = BTreeMap::new();
    let mut entries: usize = 0;
    let mut add = |c: u32, w: f64, map: &mut BTreeMap<u32, f64>| -> Result<(), TextReason> {
        entries += 1;
        if entries > W_ENTRIES_MAX {
            return Err(TextReason::FontUnsupported);
        }
        if c <= CID_MAX {
            map.insert(c, w);
        }
        Ok(())
    };
    let mut i = 0;
    while let Some(first) = items.get(i) {
        let start = cid(first).ok_or(TextReason::FontUnsupported)?;
        match items.get(i + 1).and_then(resolved) {
            Some(Object::Array(ws)) => {
                for (k, w) in ws.iter().enumerate() {
                    let w = resolved(w)
                        .and_then(number_of)
                        .ok_or(TextReason::FontUnsupported)?;
                    let k = u32::try_from(k).map_err(|_| TextReason::FontUnsupported)?;
                    let c = start.checked_add(k).ok_or(TextReason::FontUnsupported)?;
                    add(c, w, &mut map)?;
                }
                i += 2;
            }
            Some(_) => {
                let end = items
                    .get(i + 1)
                    .and_then(cid)
                    .ok_or(TextReason::FontUnsupported)?;
                let w = items
                    .get(i + 2)
                    .and_then(resolved)
                    .and_then(number_of)
                    .ok_or(TextReason::FontUnsupported)?;
                if end < start || end - start > 65_535 {
                    return Err(TextReason::FontUnsupported);
                }
                for c in start..=end {
                    add(c, w, &mut map)?;
                }
                i += 3;
            }
            None => return Err(TextReason::FontUnsupported),
        }
    }
    Ok(map.into_iter().collect())
}

/// `/CIDToGIDMap`: absent or `/Identity`, or a stream of big-endian u16 GIDs indexed by CID.
fn read_cid_to_gid<'a>(
    loader: &mut Loader<'a, '_>,
    cid_font: &'a Dictionary,
) -> Result<CidToGid, TextReason> {
    let present = cid_font
        .get(b"CIDToGIDMap")
        .is_ok_and(|m| !matches!(m, Object::Null));
    if !present {
        return Ok(CidToGid::Identity);
    }
    match loader.get(cid_font, b"CIDToGIDMap") {
        Some(Object::Name(n)) if n.as_slice() == b"Identity" => Ok(CidToGid::Identity),
        Some(Object::Stream(_)) => {
            let (id, stream) = loader
                .stream(cid_font, b"CIDToGIDMap")
                .ok_or(TextReason::FontUnsupported)?;
            let bytes = loader
                .stream_bytes(id, stream, CIDTOGID_MAX_BYTES)
                .map_err(|_| TextReason::FontUnsupported)?;
            Ok(CidToGid::Map(bytes.to_vec()))
        }
        _ => Err(TextReason::FontUnsupported),
    }
}

/// `(id, stream, is OpenType)` of the descendant's program; a program that does not match the
/// CIDFont type is `FONT_PROGRAM_UNSUPPORTED`, one that is not a stream `FONT_PROGRAM_UNREADABLE`.
fn select_program<'a>(
    loader: &Loader<'a, '_>,
    desc: Option<&'a Dictionary>,
    kind: CidKind,
) -> Result<Option<(ObjectId, &'a Stream, bool)>, TextReason> {
    let Some(d) = desc else {
        return Ok(None);
    };
    let mut found = Vec::new();
    for key in [&b"FontFile"[..], b"FontFile2", b"FontFile3"] {
        let present = d.get(key).is_ok_and(|v| !matches!(v, Object::Null));
        if present {
            let (id, stream) = loader
                .stream(d, key)
                .ok_or(TextReason::FontProgramUnreadable)?;
            found.push((key, id, stream));
        }
    }
    let [(key, id, stream)] = found.as_slice() else {
        return if found.is_empty() {
            Ok(None)
        } else {
            Err(TextReason::FontProgramUnsupported)
        };
    };
    let open_type = match (*key, kind) {
        (b"FontFile2", CidKind::Type2) => false,
        (b"FontFile3", _) => match (loader.name(&stream.dict, b"Subtype"), kind) {
            (Some(b"CIDFontType0C"), CidKind::Type0) => false,
            (Some(b"OpenType"), _) => true,
            _ => return Err(TextReason::FontProgramUnsupported),
        },
        _ => return Err(TextReason::FontProgramUnsupported),
    };
    Ok(Some((*id, *stream, open_type)))
}

/// A parsed descendant program with its CID → GID rule.
struct Glyphs<'p> {
    outlines: Outlines<'p>,
    map: GidMap<'p>,
    /// GIDs ≥ this are not glyphs (`maxp` for TrueType/OpenType, the CharStrings count for CFF).
    count: u16,
    metrics: Option<(f64, f64)>,
}

enum GidMap<'p> {
    Identity,
    Stream(&'p [u8]),
    Cids(BTreeMap<u16, u16>),
}

impl<'p> GidMap<'p> {
    fn gid(&self, cid: u32) -> Option<u16> {
        match self {
            GidMap::Identity => u16::try_from(cid).ok(),
            GidMap::Stream(bytes) => {
                let at = usize::try_from(cid).ok()?.checked_mul(2)?;
                match bytes.get(at..at.checked_add(2)?)? {
                    [hi, lo] => Some(u16::from_be_bytes([*hi, *lo])),
                    _ => None,
                }
            }
            GidMap::Cids(map) => map.get(&u16::try_from(cid).ok()?).copied(),
        }
    }

    /// CID-keyed CFF: the inverse of the charset; name-keyed: GID = CID.
    fn of_cff(outlines: &Outlines<'_>) -> GidMap<'p> {
        outlines
            .cff_layout()
            .and_then(cff_cid_to_gid)
            .map_or(GidMap::Identity, GidMap::Cids)
    }
}

impl<'p> Glyphs<'p> {
    fn new(
        bytes: &'p [u8],
        open_type: bool,
        kind: CidKind,
        cid_to_gid: Option<&'p CidToGid>,
    ) -> Option<Glyphs<'p>> {
        if open_type || kind == CidKind::Type2 {
            let face = Face::parse(bytes, 0).ok()?;
            let outlines = Outlines::of_face(&face);
            let map = match kind {
                CidKind::Type2 => match cid_to_gid {
                    Some(CidToGid::Map(m)) => GidMap::Stream(m),
                    _ => GidMap::Identity,
                },
                // CIDFontType0 in OpenType: the CFF table carries the glyphs.
                CidKind::Type0 => {
                    face.tables().cff?;
                    GidMap::of_cff(&outlines)
                }
            };
            return Some(Glyphs {
                metrics: face_metrics(&face),
                count: face.number_of_glyphs(),
                outlines,
                map,
            });
        }
        let outlines = Outlines::of_cff(bytes)?;
        let count = match &outlines {
            Outlines::Cff { table, .. } => table.number_of_glyphs(),
            _ => 0,
        };
        Some(Glyphs {
            map: GidMap::of_cff(&outlines),
            outlines,
            count,
            metrics: None,
        })
    }

    /// `(gid, drawable)`: GID ≠ 0, < the glyph count, outline present (or a whitespace glyph
    /// with a positive width); outline work is charged to `meter`.
    fn presence(
        &self,
        memo: &mut OutlineMemo,
        meter: &mut WorkMeter,
        cid: u32,
        raw: Option<&str>,
        width: f64,
    ) -> (Option<u16>, bool) {
        let blank_ok = is_whitespace_char(raw) && width > 0.0;
        match self.map.gid(cid) {
            Some(g) if g != 0 && g < self.count => {
                let drawn = memo.get(g, || self.outlines.drawn(g, meter));
                (Some(g), drawn || blank_ok)
            }
            _ => (None, false),
        }
    }
}
