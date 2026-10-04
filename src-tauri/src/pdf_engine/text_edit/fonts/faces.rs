//! Faces (SPEC §B.9.6): an exact port of `mobile/src/lib/pdf/edit/faces.ts` (family key, bold,
//! italic from the BaseFont words, then the descriptor), plus the typing surfaces built on it
//! (§B.9.1 `typing_surface` / `face_surface`: sibling fonts of one face group).

use super::{Code, FamilyHint, FontClass, FontModel, TypingSurface};
use crate::pdf_engine::text_edit::reasons::Face;
use std::sync::Arc;

const BOLD_WORDS: &[&str] = &[
    "bold",
    "semibold",
    "demibold",
    "extrabold",
    "ultrabold",
    "black",
    "heavy",
];
const ITALIC_WORDS: &[&str] = &["italic", "oblique"];
/// Not a style: dropped from both sides so `X-Regular` and `X` are one family.
const NEUTRAL_WORDS: &[&str] = &["regular", "roman", "book", "normal"];

/// Name parts of monospaced families (checked first: `LiberationMono`, `DejaVuSansMono`).
const MONO_NAMES: &[&str] = &["mono", "courier", "consol", "menlo", "monaco"];
/// Name parts of sans families (checked before the serif ones: `MicrosoftSansSerif`,
/// `CenturyGothic`).
const SANS_NAMES: &[&str] = &["sans", "gothic", "grotesk", "grotesque"];
/// Name parts of serif families.
const SERIF_NAMES: &[&str] = &[
    "serif",
    "times",
    "georgia",
    "cambria",
    "garamond",
    "bookantiqua",
    "palatino",
    "century",
    "baskerville",
    "minion",
    "caslon",
    "charter",
];

/// FontDescriptor values that speak about the face when its name says nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FaceHints {
    pub flags: Option<i64>,
    pub stem_v: Option<f64>,
    pub italic_angle: Option<f64>,
}

/// `(name without the ABCDEF+ subset tag, whether the tag was there)`.
pub fn strip_subset_tag(base: &str) -> (&str, bool) {
    let bytes = base.as_bytes();
    let tagged = bytes.len() >= 7
        && bytes
            .get(..6)
            .is_some_and(|t| t.iter().all(u8::is_ascii_uppercase))
        && bytes.get(6) == Some(&b'+');
    match (tagged, base.get(7..)) {
        (true, Some(rest)) => (rest, true),
        _ => (base, false),
    }
}

/// Splits at punctuation and where a lowercase letter or digit is followed by an uppercase one;
/// keeps the original case. `TimesNewRomanPS-BoldMT` → `Times New Roman PS Bold MT`.
fn raw_words(base: &str) -> Vec<String> {
    let (name, _) = strip_subset_tag(base);
    let mut words = Vec::new();
    let mut current = String::new();
    let mut prev: Option<char> = None;
    for ch in name.chars() {
        if !ch.is_ascii_alphanumeric() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            prev = None;
            continue;
        }
        let split = ch.is_ascii_uppercase()
            && prev.is_some_and(|p| p.is_ascii_lowercase() || p.is_ascii_digit());
        if split && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        current.push(ch);
        prev = Some(ch);
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// `wordsOf`: the BaseFont's words, lowercased.
pub fn words_of(base: &str) -> Vec<String> {
    raw_words(base)
        .into_iter()
        .map(|w| w.to_ascii_lowercase())
        .collect()
}

/// `familyOf`: the words minus style and neutral words, joined with no separator.
pub fn family_key(base: &str) -> String {
    words_of(base)
        .into_iter()
        .filter(|w| {
            let w = w.as_str();
            !BOLD_WORDS.contains(&w) && !ITALIC_WORDS.contains(&w) && !NEUTRAL_WORDS.contains(&w)
        })
        .collect()
}

/// `familyHint` (§D.2): the descriptor's FixedPitch (bit 1) or Serif (bit 2) flag, else the
/// family name. LibreOffice writes `/Flags 4` (Symbolic only) for Liberation Serif, so the flag
/// alone set every serif line in a sans face (live check B3).
pub fn family_hint(base: &str, flags: Option<i64>) -> FamilyHint {
    match flags {
        Some(f) if f & 1 != 0 => return FamilyHint::Mono,
        Some(f) if f & 2 != 0 => return FamilyHint::Serif,
        _ => {}
    }
    let name: String = strip_subset_tag(base)
        .0
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    let has = |parts: &[&str]| parts.iter().any(|p| name.contains(p));
    if has(MONO_NAMES) {
        FamilyHint::Mono
    } else if has(SANS_NAMES) {
        FamilyHint::Sans
    } else if has(SERIF_NAMES) {
        FamilyHint::Serif
    } else {
        FamilyHint::Sans
    }
}

/// A readable name for the UI: the subset tag removed, words separated by spaces.
pub fn display_name(base: &str) -> String {
    raw_words(base).join(" ")
}

fn has_word(words: &[String], set: &[&str]) -> bool {
    words.iter().any(|w| set.contains(&w.as_str()))
}

/// `isBold`: a bold word → true; a neutral word → false; ForceBold (Flags bit 19) → true; never
/// for an italic face; else `StemV ≥ 120`.
pub fn is_bold(base: &str, hints: &FaceHints) -> bool {
    let words = words_of(base);
    if has_word(&words, BOLD_WORDS) {
        return true;
    }
    if has_word(&words, NEUTRAL_WORDS) {
        return false;
    }
    if hints.flags.is_some_and(|f| f & (1 << 18) != 0) {
        return true;
    }
    if is_italic(base, hints) {
        return false;
    }
    hints.stem_v.is_some_and(|s| s >= 120.0)
}

/// `isItalic`: an italic word → true; a neutral word → false; Italic flag (bit 7) → true; else
/// `|ItalicAngle| ≥ 4`.
pub fn is_italic(base: &str, hints: &FaceHints) -> bool {
    let words = words_of(base);
    if has_word(&words, ITALIC_WORDS) {
        return true;
    }
    if has_word(&words, NEUTRAL_WORDS) {
        return false;
    }
    if hints.flags.is_some_and(|f| f & (1 << 6) != 0) {
        return true;
    }
    hints.italic_angle.is_some_and(|a| a.abs() >= 4.0)
}

fn is_type0(class: FontClass) -> bool {
    matches!(class, FontClass::Type0Cid2 | FontClass::Type0Cid0)
}

/// Word's pattern: one simple TrueType and one Type0 CIDFontType2 with the same base name.
fn word_pair(a: &FontModel, b: &FontModel) -> bool {
    match (a.class, b.class) {
        (Some(FontClass::SimpleTrueType), Some(FontClass::Type0Cid2))
        | (Some(FontClass::Type0Cid2), Some(FontClass::SimpleTrueType)) => {
            a.base_name == b.base_name
        }
        _ => false,
    }
}

/// Same class family (simple↔simple, Type0↔Type0) or the Word pair.
fn compatible(a: &FontModel, b: &FontModel) -> bool {
    match (a.class, b.class) {
        (Some(x), Some(y)) => is_type0(x) == is_type0(y) || word_pair(a, b),
        _ => false,
    }
}

fn usable(m: &FontModel) -> bool {
    m.class.is_some() && m.refusal.is_none() && !m.family_key.is_empty()
}

pub(super) fn typing_surface(
    page_fonts: &[(Vec<u8>, Arc<FontModel>)],
    primary: &[u8],
) -> TypingSurface {
    let Some((name, model)) = page_fonts.iter().find(|(n, _)| n.as_slice() == primary) else {
        return TypingSurface { fonts: Vec::new() };
    };
    let mut fonts = vec![(name.clone(), Arc::clone(model))];
    if !usable(model) {
        return TypingSurface { fonts };
    }
    for (other_name, other) in page_fonts {
        let seen = fonts.iter().any(|(_, m)| Arc::ptr_eq(m, other));
        if !seen
            && usable(other)
            && other.family_key == model.family_key
            && other.bold == model.bold
            && other.italic == model.italic
            && compatible(model, other)
        {
            fonts.push((other_name.clone(), Arc::clone(other)));
        }
    }
    TypingSurface { fonts }
}

pub(super) fn face_surface(
    page_fonts: &[(Vec<u8>, Arc<FontModel>)],
    primary: &FontModel,
    face: Face,
) -> Option<TypingSurface> {
    if !usable(primary) {
        return None;
    }
    let (bold, italic) = match face {
        Face::Regular => (false, false),
        Face::Bold => (true, false),
        Face::Italic => (false, true),
        Face::BoldItalic => (true, true),
    };
    let group: Vec<&(Vec<u8>, Arc<FontModel>)> = page_fonts
        .iter()
        .filter(|(_, m)| {
            usable(m) && m.family_key == primary.family_key && m.bold == bold && m.italic == italic
        })
        .collect();
    // The group's lead has the primary's own class family; mobile never swaps simple for Type0
    // except inside Word's simple + Type0 pair.
    let primary_type0 = primary.class.is_some_and(is_type0);
    let (lead_name, lead) = group
        .iter()
        .find(|(_, m)| m.class.is_some_and(is_type0) == primary_type0)?;
    let mut fonts = vec![(lead_name.clone(), Arc::clone(lead))];
    for (name, model) in &group {
        let seen = fonts.iter().any(|(_, m)| Arc::ptr_eq(m, model));
        if !seen && compatible(lead, model) {
            fonts.push((name.clone(), Arc::clone(model)));
        }
    }
    Some(TypingSurface { fonts })
}

impl TypingSurface {
    /// The first font (primary first) that can type `ch`, with its lowest code for it.
    pub fn writer_for(&self, ch: char) -> Option<(usize, Code)> {
        self.writer_for_with(ch, &[], &[])
    }

    /// As `writer_for`, honouring §A.3.2's preference order: a `(font index, char, code)` the run
    /// already uses for `ch`, then one the page uses, then the first font's lowest code.
    pub fn writer_for_with(
        &self,
        ch: char,
        prefer_run: &[(usize, char, Code)],
        prefer_page: &[(usize, char, Code)],
    ) -> Option<(usize, Code)> {
        for (idx, c, code) in prefer_run.iter().chain(prefer_page) {
            let usable = *c == ch
                && self
                    .fonts
                    .get(*idx)
                    .is_some_and(|(_, m)| m.can_write(ch, *code));
            if usable {
                return Some((*idx, *code));
            }
        }
        self.fonts
            .iter()
            .enumerate()
            .find_map(|(idx, (_, m))| m.code_for(ch, &[], &[]).map(|code| (idx, code)))
    }

    /// Whether some font of the surface can type U+0020 (§A.3.4 `space_mode`).
    pub fn has_space(&self) -> bool {
        self.fonts
            .iter()
            .any(|(_, m)| m.code_for(' ', &[], &[]).is_some())
    }
}

#[cfg(test)]
impl TypingSurface {
    /// Every character some font of the surface can type, sorted, without duplicates.
    pub fn alphabet(&self) -> Vec<char> {
        let mut chars: Vec<char> = self
            .fonts
            .iter()
            .flat_map(|(_, m)| m.alphabet().into_iter().map(|(c, _)| c))
            .collect();
        chars.sort_unstable();
        chars.dedup();
        chars
    }
}
