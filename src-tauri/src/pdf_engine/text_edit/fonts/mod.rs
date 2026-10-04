//! Font model (SPEC §B.9): what each code of a page font reads as, how wide it is, whether its
//! glyph really draws (program-level proof), and which characters it can type (§A.2, §A.3).
//!
//! Loading never fails: every problem becomes `FontModel::refusal` (run-level reason codes of
//! §A.10). A font whose loading ran out of the page decode budget gets `PAGE_TOO_COMPLEX` (a
//! page-level reason the walker turns into the page refusal) and is not cached, so a later page
//! with a fresh budget loads it again. All streams are read through `decode::decode_stream`
//! under the caller's `DecodeBudget`, which also pays for the glyph-proof work
//! (`glyph_budget.rs`) and the memory of each model built; nothing here panics on PDF data.

pub(crate) mod agl;
mod agl_data;
pub(crate) mod cff_encoding;
pub(crate) mod cff_layout;
mod encoding_tables;
pub(crate) mod encodings;
pub(crate) mod faces;
pub(crate) mod glyph_budget;
mod hash;
pub(crate) mod program;
pub(crate) mod simple;
pub(crate) mod std14;
mod std14_data;
pub(crate) mod tounicode;
pub(crate) mod type0;
pub(crate) mod type1;

use crate::pdf_engine::text_edit::decode::{decode_stream, DecodeBudget, DecodeError};
use crate::pdf_engine::text_edit::limits::{
    CIDTOGID_MAX_BYTES, FONT_PROGRAM_MAX_DECODED, STREAM_MAX_DECODED, TOUNICODE_MAX_DECODED,
};
use crate::pdf_engine::text_edit::reasons::{Face, TextReason};
use crate::pdf_engine::text_edit::snapshot::fnv1a_u64;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};

/// Approximate bytes of font models one `FontCache` keeps (oldest models are dropped first).
const FONT_CACHE_BYTES_MAX: usize = 128 << 20;
/// References followed for one value (a reference to a reference …).
const REF_HOPS_MAX: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontKey {
    Indirect(ObjectId),
    Direct { owner: ObjectId, name_hash: u64 },
}

impl FontKey {
    /// The key of a font dictionary written directly inside the `/Font` dictionary of `owner`.
    pub fn direct(owner: ObjectId, resource_name: &[u8]) -> FontKey {
        FontKey::Direct {
            owner,
            name_hash: fnv1a_u64(resource_name),
        }
    }
}

/// One character code: `len` 1 (simple fonts) or 2 (Identity-H), big-endian `value`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Code {
    pub value: u32,
    pub len: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FamilyHint {
    Serif,
    Sans,
    Mono,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontClass {
    SimpleTrueType,
    SimpleCff,
    SimpleOpenType,
    SimpleType1,
    Std14(std14::Std14Face),
    SimpleNonEmbedded,
    Type0Cid2,
    Type0Cid0,
}

#[derive(Debug, Clone)]
pub struct CodeInfo {
    pub text: Option<String>, // display text (ligatures expanded); None = undecodable
    pub glyph_name: Option<String>,
    pub gid: Option<u16>,
    pub width1000: Option<f64>, // glyph-space width ×1000; None = unknown
    pub drawable: bool,         // A.2 presence rule
    pub typeable_as: Option<char>, // Some(X) iff A.3.2 holds
}

#[derive(Debug, Clone)]
pub struct FontModel {
    pub key: FontKey,
    pub content_hash: u64,
    pub base_name: String,
    pub display_name: String,
    pub family_key: String,
    pub bold: bool,
    pub italic: bool,
    pub family_hint: FamilyHint,
    pub class: Option<FontClass>,
    pub refusal: Option<TextReason>,
    pub embedded: bool,
    pub subset: bool,
    pub substituted: bool,
    pub vertical: bool,
    pub ascent: f64,
    pub descent: f64,
    codes: BTreeMap<u32, CodeInfo>,
    writable: BTreeMap<char, Vec<Code>>,
    two_byte: bool,
    cid_widths: Vec<(u32, f64)>,
    default_width: f64,
    approx_bytes: usize,
}

impl FontModel {
    /// A model with no codes yet; loaders fill it and call `finish`.
    fn shell(key: FontKey, content_hash: u64, two_byte: bool) -> FontModel {
        FontModel {
            key,
            content_hash,
            base_name: String::new(),
            display_name: String::new(),
            family_key: String::new(),
            bold: false,
            italic: false,
            family_hint: FamilyHint::Sans,
            class: None,
            refusal: None,
            embedded: false,
            subset: false,
            substituted: false,
            vertical: false,
            ascent: DEFAULT_ASCENT,
            descent: DEFAULT_DESCENT,
            codes: BTreeMap::new(),
            writable: BTreeMap::new(),
            two_byte,
            cid_widths: Vec::new(),
            default_width: 0.0,
            approx_bytes: 0,
        }
    }

    /// Names, family key, faces and family hint from the BaseFont and descriptor hints.
    fn set_names(&mut self, raw_base: &str, hints: &faces::FaceHints) {
        let (base, subset) = faces::strip_subset_tag(raw_base);
        self.base_name = base.to_string();
        self.subset = subset;
        self.display_name = faces::display_name(raw_base);
        self.family_key = faces::family_key(raw_base);
        self.bold = faces::is_bold(raw_base, hints);
        self.italic = faces::is_italic(raw_base, hints);
        self.family_hint = faces::family_hint(raw_base, hints.flags);
    }

    fn refuse(&mut self, reason: TextReason) {
        self.refusal = Some(self.refusal.map_or(reason, |r| r.min(reason)));
    }

    /// Final invariants: a refused font has no class and types nothing; builds the alphabet.
    fn finish(mut self) -> FontModel {
        if self.refusal.is_some() {
            self.class = None;
            for info in self.codes.values_mut() {
                info.typeable_as = None;
            }
        }
        let len = if self.two_byte { 2 } else { 1 };
        let mut writable: BTreeMap<char, Vec<Code>> = BTreeMap::new();
        for (value, info) in &self.codes {
            if let Some(ch) = info.typeable_as {
                writable
                    .entry(ch)
                    .or_default()
                    .push(Code { value: *value, len });
            }
        }
        self.writable = writable;
        let strings: usize = self
            .codes
            .values()
            .map(|i| {
                i.text.as_ref().map_or(0, String::len)
                    + i.glyph_name.as_ref().map_or(0, String::len)
            })
            .sum();
        self.approx_bytes = 512
            + self.codes.len() * (std::mem::size_of::<CodeInfo>() + 48)
            + strings
            + self.writable.len() * 64
            + self.cid_widths.len() * 16;
        self
    }

    /// Approximate heap bytes of this model (its codes, their strings, the alphabet and the CID
    /// widths): what a page model that keeps it is charged (`walker::budget::ModelBudget::font`).
    pub fn approx_bytes(&self) -> usize {
        self.approx_bytes
    }

    fn code_len(&self) -> u8 {
        if self.two_byte {
            2
        } else {
            1
        }
    }

    /// Splits shown string bytes into codes; an odd length for a 2-byte font is
    /// `AMBIGUOUS_UNICODE`.
    pub fn split_codes(&self, bytes: &[u8]) -> Result<Vec<Code>, TextReason> {
        if !self.two_byte {
            return Ok(bytes
                .iter()
                .map(|b| Code {
                    value: u32::from(*b),
                    len: 1,
                })
                .collect());
        }
        if bytes.len() % 2 != 0 {
            return Err(TextReason::AmbiguousUnicode);
        }
        Ok(bytes
            .chunks_exact(2)
            .map(|p| match p {
                [hi, lo] => Code {
                    value: u32::from(u16::from_be_bytes([*hi, *lo])),
                    len: 2,
                },
                _ => Code { value: 0, len: 2 },
            })
            .collect())
    }

    pub fn info(&self, code: Code) -> Option<&CodeInfo> {
        if code.len != self.code_len() {
            return None;
        }
        self.codes.get(&code.value)
    }

    pub fn text(&self, code: Code) -> Option<&str> {
        self.info(code).and_then(|i| i.text.as_deref())
    }

    /// Glyph-space width ×1000 used for reading; unknown ⇒ 0.0.
    pub fn width(&self, code: Code) -> f64 {
        if code.len != self.code_len() {
            return 0.0;
        }
        if let Some(info) = self.codes.get(&code.value) {
            return info.width1000.unwrap_or(0.0);
        }
        if !self.two_byte {
            return 0.0;
        }
        match self
            .cid_widths
            .binary_search_by(|(c, _)| c.cmp(&code.value))
        {
            Ok(i) => self.cid_widths.get(i).map_or(0.0, |(_, w)| *w),
            Err(_) => self.default_width,
        }
    }

    /// `Tw` applies: a single-byte code 32.
    pub fn is_word_space(&self, code: Code) -> bool {
        code.len == 1 && code.value == 32
    }

    pub fn drawable(&self, code: Code) -> bool {
        self.info(code).is_some_and(|i| i.drawable)
    }

    /// Typeable characters with the width of their lowest code, sorted by character.
    pub fn alphabet(&self) -> Vec<(char, f64)> {
        self.writable
            .iter()
            .filter_map(|(ch, codes)| {
                let code = codes.first()?;
                Some((*ch, self.info(*code)?.width1000?))
            })
            .collect()
    }

    /// Whether `code` is one of the codes that type `ch`.
    pub fn can_write(&self, ch: char, code: Code) -> bool {
        self.writable.get(&ch).is_some_and(|c| c.contains(&code))
    }

    /// The code to write `ch` with (§A.3.2): the run's own code for it, then the page's, then the
    /// lowest.
    pub fn code_for(
        &self,
        ch: char,
        prefer_run: &[(char, Code)],
        prefer_page: &[(char, Code)],
    ) -> Option<Code> {
        let codes = self.writable.get(&ch)?;
        prefer_run
            .iter()
            .chain(prefer_page)
            .find(|(c, code)| *c == ch && codes.contains(code))
            .map(|(_, code)| *code)
            .or_else(|| codes.first().copied())
    }
}

/// Fonts already loaded for one snapshot, keyed by `FontKey`.
pub struct FontCache {
    inner: Mutex<CacheInner>,
}

#[derive(Default)]
struct CacheInner {
    map: HashMap<FontKey, Arc<FontModel>>,
    order: VecDeque<FontKey>,
    bytes: usize,
}

impl Default for FontCache {
    fn default() -> Self {
        Self::new()
    }
}

impl FontCache {
    pub fn new() -> Self {
        FontCache {
            inner: Mutex::new(CacheInner::default()),
        }
    }

    /// Never fails: every problem becomes `FontModel.refusal`.
    pub fn get_or_load(
        &self,
        doc: &Document,
        key: FontKey,
        dict: &Dictionary,
        budget: &mut DecodeBudget,
    ) -> Arc<FontModel> {
        if let Some(model) = self.lock().map.get(&key) {
            return Arc::clone(model);
        }
        let (model, cacheable) = load_font(doc, key, dict, budget);
        let model = Arc::new(model);
        if cacheable {
            let mut inner = self.lock();
            if let Some(existing) = inner.map.get(&key) {
                return Arc::clone(existing);
            }
            inner.bytes = inner.bytes.saturating_add(model.approx_bytes);
            inner.map.insert(key, Arc::clone(&model));
            inner.order.push_back(key);
            while inner.bytes > FONT_CACHE_BYTES_MAX && inner.order.len() > 1 {
                let Some(old) = inner.order.pop_front() else {
                    break;
                };
                if let Some(gone) = inner.map.remove(&old) {
                    inner.bytes = inner.bytes.saturating_sub(gone.approx_bytes);
                }
            }
        }
        model
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CacheInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub struct TypingSurface {
    pub fonts: Vec<(Vec<u8> /*resource name*/, Arc<FontModel>)>, // primary first
}

/// Group page fonts by (family_key, bold, italic); siblings = same group, not refused, same font
/// class family (simple↔simple, Type0↔Type0) or the Word pattern (one simple TrueType + one Type0
/// CIDFontType2 with the same base_name). The primary comes first.
pub fn typing_surface(page_fonts: &[(Vec<u8>, Arc<FontModel>)], primary: &[u8]) -> TypingSurface {
    faces::typing_surface(page_fonts, primary)
}

/// The sibling face group (§A.7 bold/italic) of `primary` for `face`, led by a font of the
/// primary's class family; `None` when the page has no such group.
pub fn face_surface(
    page_fonts: &[(Vec<u8>, Arc<FontModel>)],
    primary: &FontModel,
    face: Face,
) -> Option<TypingSurface> {
    faces::face_surface(page_fonts, primary, face)
}

// ---- Loading ------------------------------------------------------------------------------

const DEFAULT_ASCENT: f64 = 0.8;
const DEFAULT_DESCENT: f64 = -0.2;

fn load_font<'a>(
    doc: &'a Document,
    key: FontKey,
    dict: &'a Dictionary,
    budget: &mut DecodeBudget,
) -> (FontModel, bool) {
    let mut loader = Loader::new(doc, budget);
    let hash = hash::content_hash(&mut loader, dict);
    let subtype = loader.name(dict, b"Subtype").unwrap_or_default().to_vec();
    let model = match subtype.as_slice() {
        b"Type0" => type0::load(&mut loader, FontModel::shell(key, hash, true), dict),
        b"Type1" | b"MMType1" | b"TrueType" => simple::load(
            &mut loader,
            FontModel::shell(key, hash, false),
            dict,
            &subtype,
        ),
        b"Type3" => simple::load_type3(&mut loader, FontModel::shell(key, hash, false), dict),
        _ => {
            let mut model = FontModel::shell(key, hash, false);
            let base = loader.name(dict, b"BaseFont").unwrap_or_default();
            model.set_names(&String::from_utf8_lossy(base), &faces::FaceHints::default());
            model.refuse(TextReason::FontUnsupported);
            model
        }
    };
    let mut model = model.finish();
    // The model's memory is paid from the page budget too: a few bytes of CMap can describe
    // 65,536 codes, and a page may hold up to 256 fonts.
    if !loader.budget_hit && loader.budget.take(model.approx_bytes).is_err() {
        loader.budget_hit = true;
    }
    if loader.budget_hit {
        model.refusal = Some(TextReason::PageTooComplex);
        return (model.finish(), false);
    }
    (model, true)
}

/// Why a stream could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamFail {
    /// The page decode budget ran out (transient: the model is not cached).
    Budget,
    /// The stream itself is undecodable, too large for its role, or missing.
    Bad,
}

/// One font load: the document, the page budget and a memo of decoded streams (each stream is
/// decoded once per cap, for the content hash and its parser alike: both use the cap of the key
/// that references it, `role_cap`).
pub(crate) struct Loader<'a, 'b> {
    doc: &'a Document,
    budget: &'b mut DecodeBudget,
    decoded: HashMap<(ObjectId, usize), Result<Arc<Vec<u8>>, StreamFail>>,
    budget_hit: bool,
}

impl<'a, 'b> Loader<'a, 'b> {
    fn new(doc: &'a Document, budget: &'b mut DecodeBudget) -> Self {
        Loader {
            doc,
            budget,
            decoded: HashMap::new(),
            budget_hit: false,
        }
    }

    /// Follows references (≤ 16 hops); `None` when one dangles. Returns the last object id.
    pub(crate) fn resolve(&self, obj: &'a Object) -> Option<(Option<ObjectId>, &'a Object)> {
        let mut obj = obj;
        let mut id = None;
        for _ in 0..REF_HOPS_MAX {
            match obj {
                Object::Reference(r) => {
                    id = Some(*r);
                    obj = self.doc.objects.get(r)?;
                }
                other => return Some((id, other)),
            }
        }
        None
    }

    /// The object `id` names, resolved, with the id of the object finally reached.
    fn resolve_id(&self, id: ObjectId) -> Option<(ObjectId, &'a Object)> {
        let first = self.doc.objects.get(&id)?;
        let (last, obj) = self.resolve(first)?;
        Some((last.unwrap_or(id), obj))
    }

    /// `dict[key]`, resolved; `None` when absent, null or dangling.
    pub(crate) fn get(&self, dict: &'a Dictionary, key: &[u8]) -> Option<&'a Object> {
        let (_, obj) = self.resolve(dict.get(key).ok()?)?;
        (!matches!(obj, Object::Null)).then_some(obj)
    }

    pub(crate) fn name(&self, dict: &'a Dictionary, key: &[u8]) -> Option<&'a [u8]> {
        match self.get(dict, key)? {
            Object::Name(n) => Some(n),
            _ => None,
        }
    }

    pub(crate) fn dict(&self, dict: &'a Dictionary, key: &[u8]) -> Option<&'a Dictionary> {
        match self.get(dict, key)? {
            Object::Dictionary(d) => Some(d),
            _ => None,
        }
    }

    /// A finite number (integer or real).
    pub(crate) fn number(&self, dict: &'a Dictionary, key: &[u8]) -> Option<f64> {
        number_of(self.get(dict, key)?)
    }

    /// The stream `dict[key]` refers to, with its object id.
    pub(crate) fn stream(
        &self,
        dict: &'a Dictionary,
        key: &[u8],
    ) -> Option<(ObjectId, &'a Stream)> {
        match self.resolve(dict.get(key).ok()?)? {
            (Some(id), Object::Stream(s)) => Some((id, s)),
            _ => None,
        }
    }

    /// A meter for glyph-proof work, holding what is left of the page budget.
    pub(crate) fn work_meter(&self) -> glyph_budget::WorkMeter {
        glyph_budget::WorkMeter::new(self.budget.remaining())
    }

    /// Debits the work a meter recorded; a meter that ran out marks the load (`PAGE_TOO_COMPLEX`,
    /// not cached).
    pub(crate) fn settle(&mut self, meter: &glyph_budget::WorkMeter) {
        let used = meter.used().min(self.budget.remaining());
        if meter.exhausted() || self.budget.take(used).is_err() {
            self.budget_hit = true;
        }
    }

    /// Decoded bytes of a stream (memoised), capped at `cap` while decoding. Running out of the
    /// page budget is `Budget` (and marks the load); everything else is `Bad`.
    pub(crate) fn stream_bytes(
        &mut self,
        id: ObjectId,
        stream: &Stream,
        cap: usize,
    ) -> Result<Arc<Vec<u8>>, StreamFail> {
        let decoded = match self.decoded.get(&(id, cap)) {
            Some(result) => result.clone(),
            None => {
                let before = self.budget.remaining();
                let result = match decode_stream(stream, cap, self.budget) {
                    Ok(bytes) => Ok(Arc::new(bytes)),
                    Err(DecodeError::TooLarge) if before < cap => Err(StreamFail::Budget),
                    Err(_) => Err(StreamFail::Bad),
                };
                self.decoded.insert((id, cap), result.clone());
                result
            }
        };
        if decoded == Err(StreamFail::Budget) {
            self.budget_hit = true;
        }
        decoded
    }
}

/// The decode cap of a stream by the key that references it (§B.2): font programs, ToUnicode,
/// CIDToGIDMap, anything else.
pub(crate) fn role_cap(key: &[u8]) -> usize {
    match key {
        b"FontFile" | b"FontFile2" | b"FontFile3" => FONT_PROGRAM_MAX_DECODED,
        b"ToUnicode" => TOUNICODE_MAX_DECODED,
        b"CIDToGIDMap" => CIDTOGID_MAX_BYTES,
        _ => STREAM_MAX_DECODED,
    }
}

/// Whether a stream under `key` is read by a font parser (and so decoded once and shared with the
/// content hash).
pub(crate) fn parsed_role(key: &[u8]) -> bool {
    matches!(
        key,
        b"FontFile" | b"FontFile2" | b"FontFile3" | b"ToUnicode" | b"CIDToGIDMap"
    )
}

/// A finite number from an integer or real object. lopdf keeps reals as `f32`; widening one
/// directly would turn a written `0.001` into 0.0010000000474974513, so a real goes through its
/// shortest decimal form (what the file wrote, to f32 precision) — the value a viewer parsing the
/// text with `f64` gets.
pub(crate) fn number_of(obj: &Object) -> Option<f64> {
    let v = match obj {
        Object::Integer(i) => *i as f64,
        Object::Real(r) => r.to_string().parse::<f64>().unwrap_or(f64::from(*r)),
        _ => return None,
    };
    v.is_finite().then_some(v)
}

/// FontDescriptor values shared by every font class (§B.9.3).
#[derive(Debug, Clone, Default)]
pub(crate) struct Descriptor {
    pub flags: Option<i64>,
    pub ascent: Option<f64>,
    pub descent: Option<f64>,
    pub italic_angle: Option<f64>,
    pub stem_v: Option<f64>,
    pub missing_width: Option<f64>,
    /// `/CharSet` names; present but not a string → an empty set (no Type1 glyph is proven).
    pub charset: Option<std::collections::HashSet<String>>,
}

impl Descriptor {
    /// Reads `/FontDescriptor` of `font` (absent, null or a reference to nothing → all `None`;
    /// anything else that is not a dictionary → `FONT_UNSUPPORTED`).
    pub(crate) fn read<'a>(
        loader: &Loader<'a, '_>,
        font: &'a Dictionary,
    ) -> Result<Descriptor, TextReason> {
        let d = match loader.get(font, b"FontDescriptor") {
            None => return Ok(Descriptor::default()),
            Some(Object::Dictionary(d)) => d,
            Some(_) => return Err(TextReason::FontUnsupported),
        };
        let flags = match loader.get(d, b"Flags") {
            None => None,
            Some(Object::Integer(f)) => Some(*f),
            Some(_) => return Err(TextReason::FontUnsupported),
        };
        let charset = match loader.get(d, b"CharSet") {
            None => None,
            Some(Object::String(s, _)) => Some(type1::charset_names(s)),
            Some(_) => Some(std::collections::HashSet::new()),
        };
        Ok(Descriptor {
            flags,
            ascent: loader.number(d, b"Ascent"),
            descent: loader.number(d, b"Descent"),
            italic_angle: loader.number(d, b"ItalicAngle"),
            stem_v: loader.number(d, b"StemV"),
            missing_width: loader.number(d, b"MissingWidth"),
            charset,
        })
    }

    pub(crate) fn hints(&self) -> faces::FaceHints {
        faces::FaceHints {
            flags: self.flags,
            stem_v: self.stem_v,
            italic_angle: self.italic_angle,
        }
    }

    pub(crate) fn symbolic(&self) -> bool {
        self.flags.is_some_and(|f| f & 4 != 0)
    }
}

/// Ascent/descent in em (§A.5): descriptor (non-zero) clamped to [0.5, 1.5] / [−0.8, 0], else the
/// program's, else `fallback` (AFM), else 0.8 / −0.2.
pub(crate) fn vertical_metrics(
    desc: &Descriptor,
    program: Option<(f64, f64)>,
    fallback: Option<(f64, f64)>,
) -> (f64, f64) {
    let pick = |d: Option<f64>, p: Option<f64>, f: Option<f64>, default: f64, lo: f64, hi: f64| {
        d.filter(|v| *v != 0.0)
            .map(|v| v / 1000.0)
            .or(p.filter(|v| *v != 0.0))
            .or(f)
            .unwrap_or(default)
            .clamp(lo, hi)
    };
    (
        pick(
            desc.ascent,
            program.map(|p| p.0),
            fallback.map(|f| f.0),
            DEFAULT_ASCENT,
            0.5,
            1.5,
        ),
        pick(
            desc.descent,
            program.map(|p| p.1),
            fallback.map(|f| f.1),
            DEFAULT_DESCENT,
            -0.8,
            0.0,
        ),
    )
}

/// U+0020 and U+00A0: glyphs that may draw nothing (§A.2, §A.3.2).
pub(crate) fn is_whitespace_char(text: Option<&str>) -> bool {
    matches!(text, Some(" ") | Some("\u{a0}"))
}

#[cfg(test)]
impl FontModel {
    /// Big-endian `len` bytes of `code`.
    pub fn code_bytes(&self, code: Code) -> Vec<u8> {
        let bytes = code.value.to_be_bytes();
        let len = usize::from(code.len).min(4);
        bytes.get(4 - len..).unwrap_or_default().to_vec()
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_bounds;
#[cfg(test)]
mod tests_classes;
#[cfg(test)]
mod tests_fuzz;
#[cfg(test)]
mod tests_presence;
#[cfg(test)]
mod tests_tounicode;
#[cfg(test)]
mod tests_work;
