//! Base encodings, `/Differences`, glyph-name resolution and the character rules of SPEC §A.3:
//! display normalisation (ligatures), the reading allow-list (§A.3.3) and the typeable-character
//! exclusions (§A.3.2).

use super::agl;
use super::encoding_tables::{MAC_ROMAN_NAMES, STANDARD_NAMES, WIN_ANSI_NAMES};
use crate::pdf_engine::text_edit::reasons::TextReason;
use lopdf::Object;

/// A named base encoding a simple font may use (ISO 32000-1 Annex D.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseEncoding {
    WinAnsi,
    MacRoman,
    Standard,
}

impl BaseEncoding {
    /// `/WinAnsiEncoding`, `/MacRomanEncoding`, `/StandardEncoding`; anything else (incl.
    /// `/MacExpertEncoding`) is `UNSUPPORTED_ENCODING`.
    pub fn from_name(name: &[u8]) -> Result<BaseEncoding, TextReason> {
        match name {
            b"WinAnsiEncoding" => Ok(BaseEncoding::WinAnsi),
            b"MacRomanEncoding" => Ok(BaseEncoding::MacRoman),
            b"StandardEncoding" => Ok(BaseEncoding::Standard),
            _ => Err(TextReason::UnsupportedEncoding),
        }
    }

    pub fn table(self) -> &'static [Option<&'static str>; 256] {
        match self {
            BaseEncoding::WinAnsi => &WIN_ANSI_NAMES,
            BaseEncoding::MacRoman => &MAC_ROMAN_NAMES,
            BaseEncoding::Standard => &STANDARD_NAMES,
        }
    }

    /// The glyph name at `code`, if the encoding defines one.
    pub fn name(self, code: u8) -> Option<&'static str> {
        self.table().get(usize::from(code)).copied().flatten()
    }

    /// The 256 names as owned values (the start of a font's name table).
    pub fn names(self) -> Vec<Option<String>> {
        self.table().iter().map(|n| n.map(str::to_string)).collect()
    }
}

/// StandardEncoding's name at `code` (Type1 `seac` components, CFF built-in Standard).
pub fn standard_name(code: u8) -> Option<&'static str> {
    BaseEncoding::Standard.name(code)
}

/// The strict (Annex D) MacRoman code of a glyph name, for the TrueType `(1,0)` strategy.
pub fn mac_roman_code(name: &str) -> Option<u8> {
    static BY_NAME: std::sync::OnceLock<Vec<(&'static str, u8)>> = std::sync::OnceLock::new();
    let by_name = BY_NAME.get_or_init(|| {
        let mut pairs: Vec<(&'static str, u8)> = MAC_ROMAN_NAMES
            .iter()
            .zip(0u8..=255)
            .filter_map(|(n, code)| n.map(|n| (n, code)))
            .collect();
        // Lowest code first for a name listed twice (0x20 and 0xCA are both `space`).
        pairs.sort_by(|a, b| a.0.cmp(b.0).then(a.1.cmp(&b.1)));
        pairs.dedup_by(|later, earlier| later.0 == earlier.0);
        pairs
    });
    by_name
        .binary_search_by(|(n, _)| (*n).cmp(name))
        .ok()
        .and_then(|i| by_name.get(i))
        .map(|(_, code)| *code)
}

/// Applies a resolved `/Differences` array to a 256-entry name table. Only integers (0–255,
/// setting the next code) and names are allowed; anything else, a name before the first integer
/// or a code past 255 is `FONT_UNSUPPORTED`. A name that is not UTF-8 resolves to no name.
pub fn apply_differences(names: &mut [Option<String>], items: &[Object]) -> Result<(), TextReason> {
    let mut next: Option<usize> = None;
    for item in items {
        match item {
            Object::Integer(code) => {
                let code = usize::try_from(*code).map_err(|_| TextReason::FontUnsupported)?;
                if code > 255 {
                    return Err(TextReason::FontUnsupported);
                }
                next = Some(code);
            }
            Object::Name(bytes) => {
                let code = next.ok_or(TextReason::FontUnsupported)?;
                let slot = names.get_mut(code).ok_or(TextReason::FontUnsupported)?;
                *slot = std::str::from_utf8(bytes).ok().map(str::to_string);
                next = code.checked_add(1);
            }
            _ => return Err(TextReason::FontUnsupported),
        }
    }
    Ok(())
}

/// The Unicode scalar of a glyph name (AGL, `uniXXXX`, `uXXXX[XX]`); `.notdef` has none.
pub fn glyph_name_char(name: &str) -> Option<char> {
    if name == ".notdef" {
        return None;
    }
    agl::glyph_name_char(name)
}

/// ISO 32000-1 Annex D notes 5 and 6: the `space` glyph at code 0xA0 (WinAnsi) or 0xCA
/// (MacRoman) stands for U+00A0, and `hyphen` at 0xAD (WinAnsi) for U+00AD, so a ToUnicode that
/// says so there agrees with the glyph name (Word writes exactly this for no-break spaces and
/// soft hyphens). At any other code the names keep their own characters.
pub fn duplicate_agrees(code: u8, name: &str, text: &str) -> bool {
    matches!(
        (code, name, text),
        (0xA0 | 0xCA, "space", "\u{a0}") | (0xAD, "hyphen", "\u{ad}")
    )
}

/// Display normalisation of glyph text (§A.3.1 item 5): U+FB00–U+FB06 expand to
/// `ff fi fl ffi ffl st st`; every other character is kept.
pub fn display_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '\u{FB00}' => out.push_str("ff"),
            '\u{FB01}' => out.push_str("fi"),
            '\u{FB02}' => out.push_str("fl"),
            '\u{FB03}' => out.push_str("ffi"),
            '\u{FB04}' => out.push_str("ffl"),
            '\u{FB05}' | '\u{FB06}' => out.push_str("st"),
            other => out.push(other),
        }
    }
    out
}

/// Whether decoded text is usable at all (§A.3.1 item 5): not empty, no U+FFFD, no Private Use.
pub fn usable_text(text: &str) -> bool {
    !text.is_empty() && !text.chars().any(|c| c == '\u{FFFD}' || is_private_use(c))
}

pub fn is_private_use(ch: char) -> bool {
    matches!(u32::from(ch), 0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD)
}

/// Blocks of the reading allow-list (§A.3.3), inclusive code point ranges.
const READING_ALLOWED: &[(u32, u32)] = &[
    (0x0000, 0x024F), // Basic Latin … Latin Extended-B
    (0x0250, 0x02AF), // IPA Extensions
    (0x02B0, 0x02FF), // Spacing Modifier Letters
    (0x0370, 0x03FF), // Greek and Coptic
    (0x0400, 0x04FF), // Cyrillic
    (0x0500, 0x052F), // Cyrillic Supplement
    (0x1E00, 0x1EFF), // Latin Extended Additional
    (0x1F00, 0x1FFF), // Greek Extended
    (0x2000, 0x206F), // General Punctuation (controls removed below)
    (0x2070, 0x209F), // Superscripts and Subscripts
    (0x20A0, 0x20CF), // Currency Symbols
    (0x2100, 0x214F), // Letterlike Symbols
    (0x2150, 0x218F), // Number Forms
    (0x2190, 0x21FF), // Arrows
    (0x2200, 0x22FF), // Mathematical Operators
    (0x2500, 0x257F), // Box Drawing
    (0x25A0, 0x25FF), // Geometric Shapes
    (0x3000, 0x303F), // CJK Symbols and Punctuation
    (0x3040, 0x309F), // Hiragana
    (0x30A0, 0x30FF), // Katakana
    (0x4E00, 0x9FFF), // CJK Unified Ideographs (BMP)
    (0xAC00, 0xD7AF), // Hangul Syllables
    (0xFF00, 0xFFEF), // Halfwidth and Fullwidth Forms
    (0xFB00, 0xFB06), // Alphabetic Presentation Forms (Latin ligatures)
];

/// Right-to-left scripts and their presentation forms (§A.3.3): Hebrew, Arabic, Syriac, Thaana,
/// NKo (and the other RTL blocks between them), Hebrew/Arabic presentation forms.
const RIGHT_TO_LEFT: &[(u32, u32)] = &[
    (0x0590, 0x08FF),
    (0xFB1D, 0xFDFF),
    (0xFE70, 0xFEFE),
    (0x10800, 0x10FFF),
    (0x1E800, 0x1EFFF),
];

/// General Punctuation controls that are not part of the reading allow-list: zero-width and
/// bidi marks, separators, embeddings/overrides, and U+2060–U+206F (word joiner, invisible
/// operators, the bidi isolates U+2066–U+2069 and the deprecated format controls U+206A–U+206F).
const PUNCTUATION_CONTROLS: &[(u32, u32)] = &[
    (0x200B, 0x200F),
    (0x2028, 0x2029),
    (0x202A, 0x202E),
    (0x2060, 0x206F),
];

fn in_ranges(cp: u32, ranges: &[(u32, u32)]) -> bool {
    ranges.iter().any(|&(lo, hi)| lo <= cp && cp <= hi)
}

/// `None` when `ch` may be read (and so edited around); `RIGHT_TO_LEFT` or `COMPLEX_SCRIPT`
/// otherwise (§A.3.3).
pub fn reading_reason(ch: char) -> Option<TextReason> {
    let cp = u32::from(ch);
    if in_ranges(cp, RIGHT_TO_LEFT) {
        return Some(TextReason::RightToLeft);
    }
    if in_ranges(cp, READING_ALLOWED) && !in_ranges(cp, PUNCTUATION_CONTROLS) {
        return None;
    }
    Some(TextReason::ComplexScript)
}

/// Characters a code may never be typeable as (§A.3.2), whatever the font.
const TYPING_EXCLUDED: &[(u32, u32)] = &[
    (0x0000, 0x001F), // C0 controls
    (0x007F, 0x009F), // DEL + C1 controls
    (0x0300, 0x036F), // combining diacritical marks
    (0x1AB0, 0x1AFF),
    (0x1DC0, 0x1DFF),
    (0x20D0, 0x20FF),
    (0xFE20, 0xFE2F),
    (0x200B, 0x200F), // zero-width and bidi controls
    (0x2028, 0x2029), // line / paragraph separators
    (0x202A, 0x202E),
    (0x2060, 0x206F), // word joiner … invisible operators, bidi isolates, deprecated controls
    (0xFEFF, 0xFEFF),
    (0x180B, 0x180F), // variation selectors
    (0xFE00, 0xFE0F),
    (0xE000, 0xF8FF), // Private Use
    (0xFFFD, 0xFFFD),
    (0xFB00, 0xFB06), // ligatures are read as their letters, never typed as one character
];

/// Whether `ch` may be a typeable character (§A.3.2): BMP, not excluded, and readable (§A.3.3).
pub fn writable_char(ch: char) -> bool {
    let cp = u32::from(ch);
    if (0x20..0x7F).contains(&cp) {
        return true; // printable ASCII: in Basic Latin, in no excluded range
    }
    cp <= 0xFFFF && !in_ranges(cp, TYPING_EXCLUDED) && reading_reason(ch).is_none()
}

/// The Latin/Greek/Cyrillic allow-list for fonts the PDF reader substitutes (§A.2 row
/// "not embedded"): Latin through Extended-B, spacing modifiers, Greek, Cyrillic (+Supplement),
/// Latin Extended Additional, Greek Extended, General Punctuation, Currency and Letterlike.
const SUBSTITUTION_SAFE: &[(u32, u32)] = &[
    (0x0020, 0x024F),
    (0x02B0, 0x02FF),
    (0x0370, 0x03FF),
    (0x0400, 0x052F),
    (0x1E00, 0x1EFF),
    (0x1F00, 0x1FFF),
    (0x2000, 0x206F),
    (0x20A0, 0x20CF),
    (0x2100, 0x214F),
];

/// Whether a substituted (non-embedded, non-Standard-14) font may write `ch`.
pub fn substitution_safe(ch: char) -> bool {
    writable_char(ch) && in_ranges(u32::from(ch), SUBSTITUTION_SAFE)
}
