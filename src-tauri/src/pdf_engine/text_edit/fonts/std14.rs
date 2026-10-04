//! Standard-14 fonts (SPEC §A.2 row "Standard-14 not embedded", §B.9.2): the 12 Latin faces,
//! their aliases, and per-face AFM widths, glyph sets and vertical metrics. `Symbol` and
//! `ZapfDingbats` are recognised only to be refused (`UNSUPPORTED_ENCODING`).

use super::std14_data::{FaceMetrics, FACES, GLYPHS};
use super::FamilyHint;

/// One of the 12 Latin Core-14 faces (index order = `std14_data::FACES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Std14Face {
    Helvetica,
    HelveticaBold,
    HelveticaOblique,
    HelveticaBoldOblique,
    TimesRoman,
    TimesBold,
    TimesItalic,
    TimesBoldItalic,
    Courier,
    CourierBold,
    CourierOblique,
    CourierBoldOblique,
}

/// What a non-embedded font's BaseFont resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Std14Match {
    Latin(Std14Face),
    /// `Symbol` or `ZapfDingbats` (and their style aliases): refused `UNSUPPORTED_ENCODING`.
    Symbolic,
}

use Std14Face as S;

/// Canonical names and aliases, after `normalize` (`,` and `_` → `-`, whitespace removed, as
/// pdf.js `normalizeFontName`). The aliases are SPEC §A.2's list plus pdf.js's spellings of the
/// same three families (`getStdFontMap` in pdf.js 4.10.38: `TimesNewRomanPSMT`, `CourierNewPSMT`,
/// `Helvetica-Italic`, `Courier-Italic`, …). Narrow/Black/Unicode Arial variants are excluded:
/// their metrics are not Helvetica's.
const ALIASES: &[(&str, Std14Face)] = &[
    ("Helvetica", S::Helvetica),
    ("Helvetica-Bold", S::HelveticaBold),
    ("Helvetica-Oblique", S::HelveticaOblique),
    ("Helvetica-BoldOblique", S::HelveticaBoldOblique),
    ("Helvetica-Italic", S::HelveticaOblique),
    ("Helvetica-BoldItalic", S::HelveticaBoldOblique),
    ("Arial", S::Helvetica),
    ("ArialMT", S::Helvetica),
    ("Arial-Bold", S::HelveticaBold),
    ("Arial-BoldMT", S::HelveticaBold),
    ("Arial-Italic", S::HelveticaOblique),
    ("Arial-ItalicMT", S::HelveticaOblique),
    ("Arial-BoldItalic", S::HelveticaBoldOblique),
    ("Arial-BoldItalicMT", S::HelveticaBoldOblique),
    ("Times-Roman", S::TimesRoman),
    ("Times-Bold", S::TimesBold),
    ("Times-Italic", S::TimesItalic),
    ("Times-BoldItalic", S::TimesBoldItalic),
    ("TimesNewRoman", S::TimesRoman),
    ("TimesNewRoman-Bold", S::TimesBold),
    ("TimesNewRoman-Italic", S::TimesItalic),
    ("TimesNewRoman-BoldItalic", S::TimesBoldItalic),
    ("TimesNewRomanPS", S::TimesRoman),
    ("TimesNewRomanPSMT", S::TimesRoman),
    ("TimesNewRomanPS-Bold", S::TimesBold),
    ("TimesNewRomanPS-BoldMT", S::TimesBold),
    ("TimesNewRomanPS-Italic", S::TimesItalic),
    ("TimesNewRomanPS-ItalicMT", S::TimesItalic),
    ("TimesNewRomanPS-BoldItalic", S::TimesBoldItalic),
    ("TimesNewRomanPS-BoldItalicMT", S::TimesBoldItalic),
    ("Courier", S::Courier),
    ("Courier-Bold", S::CourierBold),
    ("Courier-Oblique", S::CourierOblique),
    ("Courier-BoldOblique", S::CourierBoldOblique),
    ("Courier-Italic", S::CourierOblique),
    ("Courier-BoldItalic", S::CourierBoldOblique),
    ("CourierNew", S::Courier),
    ("CourierNew-Bold", S::CourierBold),
    ("CourierNew-Italic", S::CourierOblique),
    ("CourierNew-BoldItalic", S::CourierBoldOblique),
    ("CourierNewPS", S::Courier),
    ("CourierNewPSMT", S::Courier),
    ("CourierNewPS-BoldMT", S::CourierBold),
    ("CourierNewPS-ItalicMT", S::CourierOblique),
    ("CourierNewPS-BoldItalicMT", S::CourierBoldOblique),
];

const SYMBOLIC: &[&str] = &[
    "Symbol",
    "Symbol-Bold",
    "Symbol-Italic",
    "Symbol-BoldItalic",
    "ZapfDingbats",
];

fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| if c == ',' || c == '_' { '-' } else { c })
        .collect()
}

/// The Standard-14 face a (subset-tag-free) BaseFont names, if any.
pub fn std14_match(base_name: &str) -> Option<Std14Match> {
    let name = normalize(base_name);
    if SYMBOLIC.contains(&name.as_str()) {
        return Some(Std14Match::Symbolic);
    }
    ALIASES
        .iter()
        .find(|(alias, _)| *alias == name)
        .map(|(_, face)| Std14Match::Latin(*face))
}

impl Std14Face {
    pub const ALL: [Std14Face; 12] = [
        S::Helvetica,
        S::HelveticaBold,
        S::HelveticaOblique,
        S::HelveticaBoldOblique,
        S::TimesRoman,
        S::TimesBold,
        S::TimesItalic,
        S::TimesBoldItalic,
        S::Courier,
        S::CourierBold,
        S::CourierOblique,
        S::CourierBoldOblique,
    ];

    fn metrics(self) -> Option<&'static FaceMetrics> {
        let index = Self::ALL.iter().position(|f| *f == self)?;
        FACES.get(index)
    }

    /// AFM advance width (glyph space ×1000) of `glyph`; `None` if the face lacks the glyph.
    pub fn width(self, glyph: &str) -> Option<f64> {
        let index = GLYPHS
            .binary_search_by(|g| g.as_bytes().cmp(glyph.as_bytes()))
            .ok()?;
        self.metrics()
            .and_then(|m| m.widths.get(index))
            .map(|w| f64::from(*w))
    }

    /// Whether the face's AFM glyph set contains `glyph` (`.notdef` never does).
    pub fn has_glyph(self, glyph: &str) -> bool {
        self.width(glyph).is_some()
    }

    /// AFM `Ascender` / 1000 (em).
    pub fn ascent(self) -> f64 {
        self.metrics()
            .map(|m| f64::from(m.ascender) / 1000.0)
            .unwrap_or(0.8)
    }

    /// AFM `Descender` / 1000 (em, ≤ 0).
    pub fn descent(self) -> f64 {
        self.metrics()
            .map(|m| f64::from(m.descender) / 1000.0)
            .unwrap_or(-0.2)
    }

    /// AFM `ItalicAngle` (degrees).
    pub fn italic_angle(self) -> f64 {
        self.metrics().map(|m| m.italic_angle).unwrap_or_default()
    }

    /// Courier → Mono, Times → Serif, Helvetica → Sans (§B.9.3).
    pub fn family_hint(self) -> FamilyHint {
        match self {
            S::Courier | S::CourierBold | S::CourierOblique | S::CourierBoldOblique => {
                FamilyHint::Mono
            }
            S::TimesRoman | S::TimesBold | S::TimesItalic | S::TimesBoldItalic => FamilyHint::Serif,
            S::Helvetica | S::HelveticaBold | S::HelveticaOblique | S::HelveticaBoldOblique => {
                FamilyHint::Sans
            }
        }
    }
}
