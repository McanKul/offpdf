//! Simple fonts (SPEC §A.2, §A.3, §B.9.3): embedded TrueType (`FontFile2`), CFF (`FontFile3
//! /Type1C`), OpenType (`FontFile3 /OpenType`), Type1 (`FontFile`), Standard-14 and other
//! non-embedded fonts. One code = one byte; every code 0–255 gets a `CodeInfo`.
//!
//! Reading: ToUnicode wins, but a code whose ToUnicode text and glyph-name text differ (after
//! ligature expansion) is undecodable. A ToUnicode that is present but unusable (decode or
//! syntax error) leaves reading to the glyph names and makes no code typeable. Writing: §A.3.2
//! (one scalar, writable character, glyph proven present in the program, width > 0).

use super::cff_encoding::{cff_builtin_encoding, CffEncoding};
use super::encodings::{
    apply_differences, display_text, duplicate_agrees, glyph_name_char, substitution_safe,
    usable_text, writable_char, BaseEncoding,
};
use super::glyph_budget::WorkMeter;
use super::program::{face_metrics, CffNames, GidLookup, OutlineMemo, Outlines, TrueTypeLookup};
use super::std14::{std14_match, Std14Face, Std14Match};
use super::tounicode::{Lookup, ToUnicode};
use super::type1::{parse_type1, GlyphProof, Type1Encoding, Type1Program};
use super::{
    is_whitespace_char, number_of, vertical_metrics, CodeInfo, Descriptor, FontClass, FontModel,
    Loader, StreamFail,
};
use crate::pdf_engine::text_edit::limits::{FONT_PROGRAM_MAX_DECODED, TOUNICODE_MAX_DECODED};
use crate::pdf_engine::text_edit::reasons::TextReason;
use lopdf::{Dictionary, Object, ObjectId, Stream};
use std::collections::{HashMap, HashSet};
use ttf_parser::{cff, Face, GlyphId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Type1,
    TrueType,
    Cff,
    OpenType,
}

/// `/Widths` with `/FirstChar` (`LastChar` = first + len − 1).
struct Widths {
    first: usize,
    values: Vec<f64>,
}

enum EncodingSpec {
    Absent,
    Named(BaseEncoding),
    Dict {
        base: Option<BaseEncoding>,
        differences: Vec<Object>,
    },
}

/// What ToUnicode a font has.
pub(crate) enum TuState {
    Absent,
    /// Present but not usable (not a stream, undecodable, too large, syntax error).
    Broken,
    Ready(ToUnicode),
}

/// Reads and parses `/ToUnicode` of `dict`.
pub(crate) fn load_tounicode<'a>(loader: &mut Loader<'a, '_>, dict: &'a Dictionary) -> TuState {
    if loader.get(dict, b"ToUnicode").is_none() {
        return if dict.has(b"ToUnicode") && !matches!(dict.get(b"ToUnicode"), Ok(Object::Null)) {
            TuState::Broken
        } else {
            TuState::Absent
        };
    }
    let Some((id, stream)) = loader.stream(dict, b"ToUnicode") else {
        return TuState::Broken;
    };
    match loader.stream_bytes(id, stream, TOUNICODE_MAX_DECODED) {
        Ok(bytes) => match super::tounicode::parse_tounicode(&bytes) {
            Ok(tu) => TuState::Ready(tu),
            Err(()) => TuState::Broken,
        },
        Err(_) => TuState::Broken,
    }
}

pub(super) fn load<'a>(
    loader: &mut Loader<'a, '_>,
    mut model: FontModel,
    dict: &'a Dictionary,
    subtype: &[u8],
) -> FontModel {
    let raw_base =
        String::from_utf8_lossy(loader.name(dict, b"BaseFont").unwrap_or_default()).into_owned();
    let truetype = subtype == b"TrueType";
    let desc = match Descriptor::read(loader, dict) {
        Ok(d) => d,
        Err(reason) => {
            model.set_names(&raw_base, &Default::default());
            model.refuse(reason);
            return model;
        }
    };
    let desc_dict = loader.dict(dict, b"FontDescriptor");
    let program = select_program(loader, desc_dict, truetype);
    let std14 = match program {
        Ok(None) => std14_match(super::faces::strip_subset_tag(&raw_base).0),
        _ => None,
    };
    let mut hints = desc.hints();
    if let Some(Std14Match::Latin(face)) = std14 {
        hints.italic_angle = hints.italic_angle.or(Some(face.italic_angle()));
    }
    model.set_names(&raw_base, &hints);
    if let Some(Std14Match::Latin(face)) = std14 {
        model.family_hint = face.family_hint();
    }
    // The structural checks all run before any of them returns, so the model carries the
    // lowest-numbered (§A.10) of the refusals that apply.
    let program = program.map_err(|reason| {
        model.embedded = true;
        model.refuse(reason);
    });
    let multiple_master = subtype == b"MMType1";
    if multiple_master {
        model.refuse(TextReason::FontUnsupported);
    }
    let widths = read_widths(loader, dict).map_err(|reason| model.refuse(reason));
    let encoding = read_encoding(loader, dict).map_err(|reason| model.refuse(reason));
    let (Ok(program), false, Ok(widths), Ok(encoding)) =
        (program, multiple_master, widths, encoding)
    else {
        return model;
    };
    model.embedded = program.is_some();
    let tu = load_tounicode(loader, dict);
    let mut parts = Parts {
        desc: &desc,
        widths: widths.as_ref(),
        encoding,
        tu,
        std14: None,
    };
    match program {
        Some((kind, id, stream)) => load_embedded(loader, &mut model, &mut parts, kind, id, stream),
        None => match std14 {
            Some(Std14Match::Latin(face)) => {
                parts.std14 = Some(face);
                model.class = Some(FontClass::Std14(face));
                let (ascent, descent) =
                    vertical_metrics(&desc, None, Some((face.ascent(), face.descent())));
                model.ascent = ascent;
                model.descent = descent;
                fill_codes(
                    &mut model,
                    &parts,
                    &Oracle::Std14(face),
                    &mut WorkMeter::new(0),
                );
            }
            Some(Std14Match::Symbolic) => model.refuse(TextReason::UnsupportedEncoding),
            None => {
                if desc.symbolic() {
                    model.refuse(TextReason::UnsupportedEncoding);
                } else if parts.widths.is_none() {
                    model.refuse(TextReason::MissingWidths);
                }
                model.class = Some(FontClass::SimpleNonEmbedded);
                model.substituted = true;
                let (ascent, descent) = vertical_metrics(&desc, None, None);
                model.ascent = ascent;
                model.descent = descent;
                fill_codes(
                    &mut model,
                    &parts,
                    &Oracle::NonEmbedded,
                    &mut WorkMeter::new(0),
                );
            }
        },
    }
    model
}

/// Type3 (refused `TYPE3`): codes still read and advance as viewers draw them, so the pen
/// position of everything after a Type3 show op stays right. `/Widths` are in glyph space and
/// scale by `/FontMatrix` (default 0.001); nothing is drawable or typeable.
pub(super) fn load_type3<'a>(
    loader: &mut Loader<'a, '_>,
    mut model: FontModel,
    dict: &'a Dictionary,
) -> FontModel {
    let raw_base =
        String::from_utf8_lossy(loader.name(dict, b"BaseFont").unwrap_or_default()).into_owned();
    model.set_names(&raw_base, &Default::default());
    model.refuse(TextReason::Type3);
    let scale = match loader.get(dict, b"FontMatrix") {
        Some(Object::Array(m)) => m
            .first()
            .and_then(|a| loader.resolve(a))
            .and_then(|(_, a)| number_of(a))
            .map(|a| a * 1000.0)
            .filter(|a| a.is_finite() && *a != 0.0),
        _ => None,
    }
    .unwrap_or(1.0);
    let widths = read_widths(loader, dict).ok().flatten().map(|w| Widths {
        first: w.first,
        values: w.values.iter().map(|v| v * scale).collect(),
    });
    let desc = Descriptor::default();
    let parts = Parts {
        desc: &desc,
        widths: widths.as_ref(),
        encoding: read_encoding(loader, dict).unwrap_or(EncodingSpec::Absent),
        tu: load_tounicode(loader, dict),
        std14: None,
    };
    fill_codes(&mut model, &parts, &Oracle::Broken, &mut WorkMeter::new(0));
    model
}

struct Parts<'d> {
    desc: &'d Descriptor,
    widths: Option<&'d Widths>,
    encoding: EncodingSpec,
    tu: TuState,
    /// The AFM face lending widths by glyph name (Standard-14, or an embedded font without
    /// `/Widths` whose BaseFont is a Standard-14 name).
    std14: Option<Std14Face>,
}

/// Per-code glyph presence for one font class.
enum Oracle<'p> {
    TrueType {
        lookup: TrueTypeLookup<'p, 'p>,
        use_names: bool,
        outlines: Outlines<'p>,
    },
    /// A bare CFF and, when the PDF gives no base encoding, the program's own one.
    Cff {
        outlines: Outlines<'p>,
        table: cff::Table<'p>,
        names: CffNames,
        builtin: Option<CffEncoding>,
    },
    Type1 {
        program: Type1Program,
        charset: Option<&'p HashSet<String>>,
    },
    Std14(Std14Face),
    NonEmbedded,
    /// The program could not be read: nothing is drawable.
    Broken,
}

fn load_embedded<'a>(
    loader: &mut Loader<'a, '_>,
    model: &mut FontModel,
    parts: &mut Parts<'_>,
    kind: Kind,
    id: ObjectId,
    stream: &'a Stream,
) {
    model.class = Some(match kind {
        Kind::Type1 => FontClass::SimpleType1,
        Kind::TrueType => FontClass::SimpleTrueType,
        Kind::Cff => FontClass::SimpleCff,
        Kind::OpenType => FontClass::SimpleOpenType,
    });
    // /Widths rule: an embedded font without them borrows AFM widths only under a Standard-14
    // name; anything else is MISSING_WIDTHS.
    let afm = match std14_match(&model.base_name) {
        Some(Std14Match::Latin(face)) => Some(face),
        _ => None,
    };
    if parts.widths.is_none() {
        match afm {
            Some(face) => parts.std14 = Some(face),
            None => model.refuse(TextReason::MissingWidths),
        }
    }
    let fallback = afm.map(|f| (f.ascent(), f.descent()));
    let (ascent, descent) = vertical_metrics(parts.desc, None, fallback);
    model.ascent = ascent;
    model.descent = descent;
    let bytes = match loader.stream_bytes(id, stream, FONT_PROGRAM_MAX_DECODED) {
        Ok(b) => b,
        Err(StreamFail::Budget | StreamFail::Bad) => {
            model.refuse(TextReason::FontProgramUnreadable);
            fill_codes(model, parts, &Oracle::Broken, &mut WorkMeter::new(0));
            return;
        }
    };
    let face = match kind {
        Kind::TrueType | Kind::OpenType => Face::parse(&bytes, 0).ok(),
        Kind::Cff | Kind::Type1 => None,
    };
    let needs_builtin = match &parts.encoding {
        EncodingSpec::Absent => true,
        EncodingSpec::Dict { base, .. } => base.is_none(),
        EncodingSpec::Named(_) => false,
    };
    let oracle = match kind {
        Kind::TrueType | Kind::OpenType => match face.as_ref() {
            Some(face) => {
                let (ascent, descent) = vertical_metrics(parts.desc, face_metrics(face), fallback);
                model.ascent = ascent;
                model.descent = descent;
                // PDF 32000 §9.6.6.4 / ISO 32000-2 §9.6.5.4: the Symbolic flag selects glyphs by
                // code through the (3,0)/(1,0) cmaps (an /Encoding then only names the codes).
                let use_names = !parts.desc.symbolic();
                let outlines = Outlines::of_face(face);
                Oracle::TrueType {
                    lookup: TrueTypeLookup::new(face, &outlines),
                    use_names,
                    outlines,
                }
            }
            None => Oracle::Broken,
        },
        Kind::Cff => {
            let builtin = if needs_builtin {
                cff_builtin_encoding(&bytes).map(Some)
            } else {
                Ok(None)
            };
            match (Outlines::of_cff(&bytes), builtin) {
                (Some(outlines), Ok(builtin)) => {
                    let found = match &outlines {
                        Outlines::Cff { guard, table } => {
                            Some((CffNames::new(guard.layout(), table), *table))
                        }
                        _ => None,
                    };
                    match found {
                        Some((names, table)) => Oracle::Cff {
                            outlines,
                            table,
                            names,
                            builtin,
                        },
                        None => Oracle::Broken,
                    }
                }
                _ => Oracle::Broken,
            }
        }
        Kind::Type1 => {
            let length = |key: &[u8]| {
                loader
                    .get(&stream.dict, key)
                    .and_then(number_of)
                    .filter(|v| {
                        *v >= 0.0 && v.fract() == 0.0 && *v <= FONT_PROGRAM_MAX_DECODED as f64
                    })
                    .map(|v| v as usize)
            };
            match (length(b"Length1"), length(b"Length2")) {
                (Some(l1), Some(l2)) => match parse_type1(&bytes, l1, l2) {
                    Ok(program) => Oracle::Type1 {
                        program,
                        charset: parts.desc.charset.as_ref(),
                    },
                    Err(()) => Oracle::Broken,
                },
                _ => Oracle::Broken,
            }
        }
    };
    if let Oracle::TrueType { outlines, .. } | Oracle::Cff { outlines, .. } = &oracle {
        if let Some(reason) = outlines.refusal() {
            model.refuse(reason);
        }
    }
    match &oracle {
        Oracle::Broken => model.refuse(TextReason::FontProgramUnreadable),
        Oracle::TrueType {
            use_names: false, ..
        } if matches!(parts.encoding, EncodingSpec::Absent) => match parts.tu {
            // A symbolic TrueType font without /Encoding has no glyph names: its text can only
            // come from ToUnicode.
            TuState::Absent => model.refuse(TextReason::NoTounicode),
            TuState::Broken => model.refuse(TextReason::AmbiguousUnicode),
            TuState::Ready(_) => {}
        },
        _ => {}
    }
    let mut meter = loader.work_meter();
    fill_codes(model, parts, &oracle, &mut meter);
    loader.settle(&meter);
}

/// `(kind, object id, stream)` of the descriptor's program; a program that does not match the
/// font type is `FONT_PROGRAM_UNSUPPORTED`, one that is not a stream `FONT_PROGRAM_UNREADABLE`.
#[allow(clippy::type_complexity)]
fn select_program<'a>(
    loader: &Loader<'a, '_>,
    desc: Option<&'a Dictionary>,
    truetype: bool,
) -> Result<Option<(Kind, ObjectId, &'a Stream)>, TextReason> {
    let Some(d) = desc else {
        return Ok(None);
    };
    let mut found = Vec::new();
    for key in [&b"FontFile"[..], b"FontFile2", b"FontFile3"] {
        let present = d.get(key).is_ok_and(|v| !matches!(v, Object::Null));
        if !present {
            continue;
        }
        let (id, stream) = loader
            .stream(d, key)
            .ok_or(TextReason::FontProgramUnreadable)?;
        found.push((key, id, stream));
    }
    let [(key, id, stream)] = found.as_slice() else {
        return if found.is_empty() {
            Ok(None)
        } else {
            Err(TextReason::FontProgramUnsupported)
        };
    };
    let kind = match (*key, truetype) {
        (b"FontFile", false) => Kind::Type1,
        (b"FontFile2", true) => Kind::TrueType,
        (b"FontFile3", _) => match (loader.name(&stream.dict, b"Subtype"), truetype) {
            (Some(b"Type1C"), false) => Kind::Cff,
            (Some(b"OpenType"), _) => Kind::OpenType,
            _ => return Err(TextReason::FontProgramUnsupported),
        },
        _ => return Err(TextReason::FontProgramUnsupported),
    };
    Ok(Some((kind, *id, *stream)))
}

/// `/Widths` + `/FirstChar` + `/LastChar`: integers 0–255, `LastChar ≥ FirstChar`, exactly
/// `LastChar − FirstChar + 1` finite numbers; anything else is `FONT_UNSUPPORTED`.
fn read_widths<'a>(
    loader: &Loader<'a, '_>,
    dict: &'a Dictionary,
) -> Result<Option<Widths>, TextReason> {
    let present = dict
        .get(b"Widths")
        .is_ok_and(|w| !matches!(w, Object::Null));
    if !present {
        return Ok(None);
    }
    let Some(Object::Array(items)) = loader.get(dict, b"Widths") else {
        return Err(TextReason::FontUnsupported);
    };
    let int = |key: &[u8]| match loader.get(dict, key) {
        Some(Object::Integer(v)) if (0..=255).contains(v) => usize::try_from(*v).ok(),
        _ => None,
    };
    let (Some(first), Some(last)) = (int(b"FirstChar"), int(b"LastChar")) else {
        return Err(TextReason::FontUnsupported);
    };
    if last < first || items.len() != last - first + 1 {
        return Err(TextReason::FontUnsupported);
    }
    let values = items
        .iter()
        .map(|item| loader.resolve(item).and_then(|(_, o)| number_of(o)))
        .collect::<Option<Vec<f64>>>()
        .ok_or(TextReason::FontUnsupported)?;
    Ok(Some(Widths { first, values }))
}

/// `/Encoding`: a base-encoding name, or a dictionary with an optional `/BaseEncoding` and a
/// `/Differences` array that must be well-formed (checked here, applied in `name_table`).
fn read_encoding<'a>(
    loader: &Loader<'a, '_>,
    dict: &'a Dictionary,
) -> Result<EncodingSpec, TextReason> {
    let present = dict
        .get(b"Encoding")
        .is_ok_and(|e| !matches!(e, Object::Null));
    if !present {
        return Ok(EncodingSpec::Absent);
    }
    match loader.get(dict, b"Encoding") {
        Some(Object::Name(name)) => Ok(EncodingSpec::Named(BaseEncoding::from_name(name)?)),
        Some(Object::Dictionary(enc)) => {
            let base = match loader.get(enc, b"BaseEncoding") {
                None => None,
                Some(Object::Name(name)) => Some(BaseEncoding::from_name(name)?),
                Some(_) => return Err(TextReason::FontUnsupported),
            };
            let differences = match loader.get(enc, b"Differences") {
                None => Vec::new(),
                Some(Object::Array(items)) => items.clone(),
                Some(_) => return Err(TextReason::FontUnsupported),
            };
            apply_differences(&mut vec![None; 256], &differences)?;
            Ok(EncodingSpec::Dict { base, differences })
        }
        _ => Err(TextReason::FontUnsupported),
    }
}

/// Glyph names per code plus, for a CFF built-in custom encoding, the GID each code maps to.
fn name_table(parts: &Parts<'_>, oracle: &Oracle<'_>) -> (Vec<Option<String>>, Vec<Option<u16>>) {
    let mut names: Vec<Option<String>> = vec![None; 256];
    let mut direct: Vec<Option<u16>> = vec![None; 256];
    let builtin = |names: &mut Vec<Option<String>>, direct: &mut Vec<Option<u16>>| match oracle {
        Oracle::Type1 { program, .. } => match &program.builtin {
            Some(Type1Encoding::Standard) => *names = BaseEncoding::Standard.names(),
            Some(Type1Encoding::Custom(map)) => {
                for (code, name) in map {
                    if let Some(slot) = names.get_mut(usize::from(*code)) {
                        *slot = Some(name.clone());
                    }
                }
            }
            None => {}
        },
        Oracle::Cff {
            table,
            builtin,
            outlines,
            ..
        } => match builtin {
            Some(CffEncoding::Standard) => *names = BaseEncoding::Standard.names(),
            Some(CffEncoding::Custom(map)) => {
                // GID → SID from one walk of the charset (ttf-parser's `glyph_name` walks a
                // format 1/2 charset per call); the predefined Expert charsets have no walk.
                let sids = outlines
                    .cff_layout()
                    .filter(|layout| !layout.is_cid())
                    .and_then(|layout| Some((layout, layout.gid_to_sid()?)));
                for (code, gid) in map {
                    if *gid >= table.number_of_glyphs() {
                        continue;
                    }
                    let name = match &sids {
                        Some((layout, sids)) => sids
                            .get(usize::from(*gid))
                            .copied()
                            .flatten()
                            .and_then(|sid| layout.sid_name(sid)),
                        None => table.glyph_name(GlyphId(*gid)),
                    };
                    let i = usize::from(*code);
                    if let (Some(n), Some(d)) = (names.get_mut(i), direct.get_mut(i)) {
                        *n = name.map(str::to_string);
                        *d = Some(*gid);
                    }
                }
            }
            // Expert: no code is typeable (readable through ToUnicode only).
            Some(CffEncoding::Expert) | None => {}
        },
        // Symbolic TrueType without /Encoding: codes go through the cmap, not names.
        Oracle::TrueType {
            use_names: false, ..
        }
        | Oracle::Broken => {}
        Oracle::TrueType { .. } | Oracle::Std14(_) | Oracle::NonEmbedded => {
            *names = BaseEncoding::Standard.names();
        }
    };
    match &parts.encoding {
        EncodingSpec::Absent => builtin(&mut names, &mut direct),
        EncodingSpec::Named(base) => names = base.names(),
        EncodingSpec::Dict { base, differences } => {
            match base {
                Some(base) => names = base.names(),
                None => builtin(&mut names, &mut direct),
            }
            let before = names.clone();
            if apply_differences(&mut names, differences).is_err() {
                names = before.clone(); // validated in read_encoding; never taken
            }
            for ((old, new), slot) in before.iter().zip(&names).zip(direct.iter_mut()) {
                if old != new {
                    *slot = None; // a /Differences name replaces the built-in GID
                }
            }
        }
    }
    (names, direct)
}

/// Presence results of one font load: outlines per GID, Type1 proofs per glyph name.
#[derive(Default)]
struct Memo {
    outlines: OutlineMemo,
    proofs: HashMap<String, GlyphProof>,
}

/// Builds the `CodeInfo` of every code 0–255; glyph work is charged to `meter`.
fn fill_codes(
    model: &mut FontModel,
    parts: &Parts<'_>,
    oracle: &Oracle<'_>,
    meter: &mut WorkMeter,
) {
    let (names, direct) = name_table(parts, oracle);
    let tu = match &parts.tu {
        TuState::Ready(t) => Some(t),
        TuState::Absent | TuState::Broken => None,
    };
    let tu_broken = matches!(parts.tu, TuState::Broken);
    let substituted = matches!(oracle, Oracle::NonEmbedded);
    let mut memo = Memo::default();
    let mut infos = Vec::with_capacity(256);
    for code in 0u8..=255 {
        let i = usize::from(code);
        let name = names.get(i).cloned().flatten();
        let width = width_of(parts, code, name.as_deref());
        let mapped = tu.map(|t| t.lookup(u32::from(code)));
        if name.is_none() && width.is_none() && matches!(mapped, None | Some(Lookup::Absent)) {
            continue; // nothing to read, measure or type: `info` is None, `width` 0
        }
        let name_char = name.as_deref().and_then(glyph_name_char);
        let name_text = name_char.map(String::from);
        let raw = match mapped {
            Some(Lookup::Undecodable) => None,
            Some(Lookup::Text(t)) => match (&name_text, name.as_deref()) {
                (Some(_), Some(n)) if duplicate_agrees(code, n, t) => Some(t.to_string()),
                (Some(n), _) if display_text(t) != display_text(n) => None,
                _ => Some(t.to_string()),
            },
            Some(Lookup::Absent) | None => name_text,
        };
        let text = raw.as_deref().filter(|t| usable_text(t)).map(display_text);
        let ws = is_whitespace_char(text.as_deref());
        let d = direct.get(i).copied().flatten();
        let glyph = GlyphRef {
            code,
            name: name.as_deref(),
            name_char,
            direct: d,
        };
        let (gid, drawable) = presence(oracle, &mut memo, meter, glyph, ws, width);
        let single = raw.as_deref().and_then(|t| {
            let mut chars = t.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Some(c),
                _ => None,
            }
        });
        let typeable_as = single.filter(|ch| {
            drawable
                && width.is_some_and(|w| w > 0.0)
                && !tu_broken
                && text.is_some()
                && writable_char(*ch)
                && (!substituted || substitution_safe(*ch))
        });
        infos.push((
            u32::from(code),
            CodeInfo {
                text,
                glyph_name: name,
                gid,
                width1000: width,
                drawable,
                typeable_as,
            },
        ));
    }
    // Codes ascend: the map is built in bulk, not by 256 single inserts.
    model.codes = infos.into_iter().collect();
}

/// `/Widths[code − FirstChar]` inside the range, `/MissingWidth` outside it only when present;
/// without `/Widths`, the AFM width of the glyph name (Standard-14).
fn width_of(parts: &Parts<'_>, code: u8, name: Option<&str>) -> Option<f64> {
    match parts.widths {
        Some(w) => usize::from(code)
            .checked_sub(w.first)
            .and_then(|i| w.values.get(i).copied())
            .or(parts.desc.missing_width),
        None => parts.std14.zip(name).and_then(|(face, n)| face.width(n)),
    }
}

/// One code and what its encoding says: glyph name (and its AGL character), or a GID straight
/// from a CFF built-in encoding.
#[derive(Clone, Copy)]
struct GlyphRef<'n> {
    code: u8,
    name: Option<&'n str>,
    name_char: Option<char>,
    direct: Option<u16>,
}

/// `(gid, drawable)` of one code under the font class's presence rule (§A.2).
fn presence(
    oracle: &Oracle<'_>,
    memo: &mut Memo,
    meter: &mut WorkMeter,
    glyph: GlyphRef<'_>,
    whitespace: bool,
    width: Option<f64>,
) -> (Option<u16>, bool) {
    let GlyphRef {
        code,
        name,
        name_char,
        direct,
    } = glyph;
    let blank_ok = whitespace && width.is_some_and(|w| w > 0.0);
    match oracle {
        Oracle::TrueType {
            lookup,
            use_names,
            outlines,
        } => {
            let found = if *use_names {
                name.map_or(GidLookup::None, |n| lookup.by_name_char(n, name_char))
            } else {
                lookup.symbolic(code)
            };
            match found {
                GidLookup::Agree(g) if g != 0 && g < lookup.face.number_of_glyphs() => {
                    let drawn = memo.outlines.get(g, || outlines.drawn(g, meter));
                    (Some(g), drawn || blank_ok)
                }
                _ => (None, false),
            }
        }
        Oracle::Cff {
            outlines,
            table,
            names,
            ..
        } => {
            let gid =
                direct.or_else(|| name.filter(|n| *n != ".notdef").and_then(|n| names.gid(n)));
            match gid {
                Some(g) if g != 0 && g < table.number_of_glyphs() => {
                    let drawn = memo.outlines.get(g, || outlines.drawn(g, meter));
                    (Some(g), drawn || blank_ok)
                }
                _ => (None, false),
            }
        }
        Oracle::Type1 { program, charset } => {
            let Some(n) = name else {
                return (None, false);
            };
            if charset.is_some_and(|set| !set.contains(n)) {
                return (None, false);
            }
            let proof = match memo.proofs.get(n) {
                Some(proof) => *proof,
                None => {
                    let proof = program.proof(n, meter);
                    memo.proofs.insert(n.to_string(), proof);
                    proof
                }
            };
            let drawn = match proof {
                GlyphProof::Drawn => true,
                GlyphProof::Blank { width: w } => blank_ok && w > 0.0,
                GlyphProof::Unproven => false,
            };
            (None, drawn)
        }
        Oracle::Std14(face) => (None, name.is_some_and(|n| face.has_glyph(n))),
        Oracle::NonEmbedded => (None, width.is_some_and(|w| w > 0.0)),
        Oracle::Broken => (None, false),
    }
}
