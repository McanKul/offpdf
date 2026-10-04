//! Reason codes, edit problems, warnings and every `AppError` the text-edit path builds
//! (SPEC §A.10, §B.3, copy in §D.11.3 and §D.11.6). The canonical code list is
//! `src/lib/editor/text-reasons.json`, cross-locked by `reasons_json_matches_enums`.

use crate::error::AppError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TextReason {
    // run level, in priority order (A.10)
    NestedForm,
    SplitContent,
    SharedContent,
    InlineImage,
    InvisibleText,
    TextClipMode,
    ZeroSize,
    Vertical,
    MirroredText,
    RotatedText,
    SkewedText,
    Clipped,
    OptionalContent,
    SoftMask,
    Pattern,
    ActualText,
    MissingFont,
    Type3,
    FontUnsupported,
    FontNotEmbedded,
    FontProgramUnsupported,
    FontProgramUnreadable,
    UnsupportedEncoding,
    MissingWidths,
    NoTounicode,
    AmbiguousUnicode,
    RightToLeft,
    ComplexScript,
    DuplicateText,
    PerGlyphText,
    NoWritableGlyphs,
    // page level
    MalformedContent,
    UnsupportedFilter,
    PageTooComplex,
    Geometry,
    // image occurrences (classifier)
    MaskedImage,
    SharedXobject,
    TransformedImage,
}

use TextReason as R;

impl TextReason {
    /// Page-level codes: every run on the page is refused with it.
    pub const PAGE: &'static [TextReason] = &[
        R::MalformedContent,
        R::UnsupportedFilter,
        R::PageTooComplex,
        R::Geometry,
    ];
    /// Exact SCREAMING_SNAKE spelling, e.g. "TYPE3", "NO_TOUNICODE", "SHARED_XOBJECT".
    pub fn as_str(self) -> &'static str {
        match self {
            R::NestedForm => "NESTED_FORM",
            R::SplitContent => "SPLIT_CONTENT",
            R::SharedContent => "SHARED_CONTENT",
            R::InlineImage => "INLINE_IMAGE",
            R::InvisibleText => "INVISIBLE_TEXT",
            R::TextClipMode => "TEXT_CLIP_MODE",
            R::ZeroSize => "ZERO_SIZE",
            R::Vertical => "VERTICAL",
            R::MirroredText => "MIRRORED_TEXT",
            R::RotatedText => "ROTATED_TEXT",
            R::SkewedText => "SKEWED_TEXT",
            R::Clipped => "CLIPPED",
            R::OptionalContent => "OPTIONAL_CONTENT",
            R::SoftMask => "SOFT_MASK",
            R::Pattern => "PATTERN",
            R::ActualText => "ACTUAL_TEXT",
            R::MissingFont => "MISSING_FONT",
            R::Type3 => "TYPE3",
            R::FontUnsupported => "FONT_UNSUPPORTED",
            R::FontNotEmbedded => "FONT_NOT_EMBEDDED",
            R::FontProgramUnsupported => "FONT_PROGRAM_UNSUPPORTED",
            R::FontProgramUnreadable => "FONT_PROGRAM_UNREADABLE",
            R::UnsupportedEncoding => "UNSUPPORTED_ENCODING",
            R::MissingWidths => "MISSING_WIDTHS",
            R::NoTounicode => "NO_TOUNICODE",
            R::AmbiguousUnicode => "AMBIGUOUS_UNICODE",
            R::RightToLeft => "RIGHT_TO_LEFT",
            R::ComplexScript => "COMPLEX_SCRIPT",
            R::DuplicateText => "DUPLICATE_TEXT",
            R::PerGlyphText => "PER_GLYPH_TEXT",
            R::NoWritableGlyphs => "NO_WRITABLE_GLYPHS",
            R::MalformedContent => "MALFORMED_CONTENT",
            R::UnsupportedFilter => "UNSUPPORTED_FILTER",
            R::PageTooComplex => "PAGE_TOO_COMPLEX",
            R::Geometry => "GEOMETRY",
            R::MaskedImage => "MASKED_IMAGE",
            R::SharedXobject => "SHARED_XOBJECT",
            R::TransformedImage => "TRANSFORMED_IMAGE",
        }
    }

    /// True for the four page-level codes.
    pub fn is_page_level(self) -> bool {
        Self::PAGE.contains(&self)
    }

    /// `(title, body)` of the reason copy (§D.11.3); codes without a row use the "(unknown)" row.
    pub fn copy(self) -> (&'static str, &'static str) {
        match self {
            R::NestedForm => ("Part of a reused block", "This text is inside a block the document can reuse, so changing it here could change it in other places too."),
            R::SplitContent => ("Stored in two pieces", "The instructions that draw this line are split across two parts of the page."),
            R::SharedContent => ("Shared with other pages", "This part of the page is shared with other pages, so a change here would change them too."),
            R::InlineImage => ("After an unreadable picture", "A picture stored inside this page can't be measured exactly, so text drawn after it can't be changed safely."),
            R::InvisibleText => ("Hidden text", "This is hidden text, such as the searchable layer of a scan. Changing it would change nothing you can see."),
            R::TextClipMode => ("Used as a shape", "This text is used as a shape that clips other content, so it can't be changed safely."),
            R::ZeroSize => ("No visible size", "This text is drawn at zero size."),
            R::Vertical => ("Vertical text", "This text runs top to bottom. Only lines that read across the page can be changed."),
            R::MirroredText => ("Mirrored text", "This text is drawn mirrored, so new letters can't be placed the same way."),
            R::RotatedText => ("Turned at an angle", "This line doesn't run straight across the page as shown. Only level lines can be changed."),
            R::SkewedText => ("Tilted line", "This line's baseline is tilted by the page layout, so it can't be changed safely."),
            R::Clipped => ("Partly cut off", "Part of this text is cut off by the page edge or a clipping area, so a change might not show."),
            R::OptionalContent => ("On a layer", "This text is on a layer that is hidden or depends on viewer settings, so a change might not show."),
            R::SoftMask => ("Masked text", "This text is drawn through a transparency mask, so a change could look different from what you type."),
            R::Pattern => ("Pattern fill", "This text is filled with a pattern or gradient rather than a colour."),
            R::ActualText => ("Separate reading copy", "The document keeps a separate copy of this text for search and screen readers. Changing only the visible letters would make the two disagree."),
            R::MissingFont => ("Font missing", "The font this text uses is missing from the document."),
            R::Type3 => ("Font made of drawings", "This text uses a font drawn from shapes, which OffPDF can't type with."),
            R::FontUnsupported => ("Unusual font setup", "This font is set up in a way OffPDF can't change safely."),
            R::FontNotEmbedded => ("Font not included", "This font isn't included in the PDF, so OffPDF can't check which letters it can draw."),
            R::FontProgramUnsupported => ("Font type not supported yet", "This text uses a kind of embedded font that OffPDF can't check yet."),
            R::FontProgramUnreadable => ("Font can't be read", "The font included in this PDF can't be read, so OffPDF can't check which letters it can draw."),
            R::UnsupportedEncoding => ("Letter mapping not supported", "This font maps letters in a way OffPDF can't write yet."),
            R::MissingWidths => ("Letter widths missing", "The document doesn't say how wide this font's letters are, so the rest of the line couldn't be kept in place."),
            R::NoTounicode => ("Letters not identified", "The document doesn't say which letters this font draws, so OffPDF can't read or retype them."),
            R::AmbiguousUnicode => ("Letters can't be confirmed", "Some letters in this line can't be read with certainty, so OffPDF can't show the current text reliably."),
            R::RightToLeft => ("Right-to-left script", "The document has already ordered and shaped these letters, and changing them would undo that work."),
            R::ComplexScript => ("Shaped script", "This script joins or reorders letters, and the document has already done that shaping. OffPDF can't redo it yet."),
            R::DuplicateText => ("Drawn twice", "This text is drawn twice (for example as a shadow or to look bolder), so changing one copy would leave the other behind."),
            R::PerGlyphText => ("Letters placed one by one", "This page places every letter on its own, so a line can't be changed as one piece."),
            R::NoWritableGlyphs => ("No letters to type with", "The font this line uses has no letters OffPDF can confirm, so nothing can be typed with it."),
            R::MalformedContent => ("Damaged page content", "OffPDF couldn't read this page's content completely, so none of its text can be changed."),
            R::UnsupportedFilter => ("Unusual compression", "This page's content is stored or compressed in a way OffPDF can't check."),
            R::PageTooComplex => ("Too complex to check", "This page has too much content to check safely."),
            R::Geometry => ("Custom page unit", "This page uses a custom unit size or unreadable page boxes, which OffPDF doesn't edit yet."),
            R::MaskedImage | R::SharedXobject | R::TransformedImage => {
                ("Can't be changed safely", "OffPDF can't change this text safely.")
            }
        }
    }

    /// Run codes (and image codes) → `TEXT_EDIT_REFUSED` naming the reason; page codes → an
    /// error whose code is the page code itself.
    pub fn to_app_error(self, page_number: Option<u32>) -> AppError {
        let (title, body) = self.copy();
        if self.is_page_level() {
            return AppError::new(self.as_str(), title, on_page_colon(page_number, body))
                .with_suggestion("Restore the original text on that page.")
                .with_details(detail_line(page_number, self.as_str()));
        }
        AppError::new(
            EditProblemCode::TextEditRefused.as_str(),
            "This text can't be changed safely",
            on_page_colon(page_number, body),
        )
        .with_suggestion("Restore the original text for that line.")
        .with_details(detail_line(page_number, self.as_str()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EditProblemCode {
    GlyphMissing,
    SpaceNotWritable,
    InvalidText,
    TextTooLong,
    TextOutsideVisibleArea,
    FaceUnavailable,
    StyleUnavailable,
    EditConflict,
    TextEditRefused,
    Stale,
    PenDrift,
    StateChanged,
    EditVerifyFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TextWarningCode {
    NextTextOverlap,
    EditNotVisible,
    PreviewUnavailable,
}

/// Which control a `STYLE_UNAVAILABLE` refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StyleField {
    Size,
    Face,
    Colour,
}

/// A face of a font family (lives here because `EditProblem` needs it before `fonts/`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Face {
    Regular,
    Bold,
    Italic,
    BoldItalic,
}

impl Face {
    fn adjective(self) -> &'static str {
        match self {
            Face::Regular => "regular",
            Face::Bold => "bold",
            Face::Italic => "italic",
            Face::BoldItalic => "bold italic",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EditProblem {
    pub code: EditProblemCode,
    pub chars: Vec<char>, // GLYPH_MISSING / FACE_UNAVAILABLE characters, typing order
    pub reason: Option<TextReason>, // TEXT_EDIT_REFUSED: the run's reason
    pub face: Option<Face>, // FACE_UNAVAILABLE: the requested face
    pub field: Option<StyleField>, // STYLE_UNAVAILABLE: which control
    pub detail: Option<String>, // technical detail (check id, drift, byte offset)
}

impl EditProblem {
    /// A problem with only a code and an optional technical detail.
    pub fn new(code: EditProblemCode, detail: Option<String>) -> EditProblem {
        EditProblem {
            code,
            chars: Vec::new(),
            reason: None,
            face: None,
            field: None,
            detail,
        }
    }
}

pub struct ProblemCtx<'a> {
    pub page_number: Option<u32>,
    pub file_name: Option<&'a str>,
    pub face: Option<Face>,
    pub reason: Option<TextReason>,
}

use EditProblemCode as P;

impl EditProblemCode {
    pub fn as_str(self) -> &'static str {
        match self {
            P::GlyphMissing => "GLYPH_MISSING",
            P::SpaceNotWritable => "SPACE_NOT_WRITABLE",
            P::InvalidText => "INVALID_TEXT",
            P::TextTooLong => "TEXT_TOO_LONG",
            P::TextOutsideVisibleArea => "TEXT_OUTSIDE_VISIBLE_AREA",
            P::FaceUnavailable => "FACE_UNAVAILABLE",
            P::StyleUnavailable => "STYLE_UNAVAILABLE",
            P::EditConflict => "EDIT_CONFLICT",
            P::TextEditRefused => "TEXT_EDIT_REFUSED",
            P::Stale => "STALE",
            P::PenDrift => "PEN_DRIFT",
            P::StateChanged => "STATE_CHANGED",
            P::EditVerifyFailed => "EDIT_VERIFY_FAILED",
        }
    }

    /// The Save-time `AppError` for this problem (copy in §D.11.6).
    pub fn to_app_error(self, p: &EditProblem, ctx: &ProblemCtx<'_>) -> AppError {
        let n = ctx.page_number;
        let (title, message, suggestion): (&str, String, &str) = match self {
            P::GlyphMissing => (
                "The font can't draw some letters",
                format!("{}, the document's font can't draw: {}", on_page(n), char_list(&p.chars)),
                "Use other characters, or add a new text box with Add text.",
            ),
            P::SpaceNotWritable => (
                "A space can't go there",
                format!("{}, spaces in the changed line are gaps between letters, so a space can only go between two letters, one at a time.", on_page(n)),
                "Remove the extra space.",
            ),
            P::InvalidText => (
                "These characters can't be added",
                "Line breaks, tabs and control characters can't be added. Each box is a single line.".to_string(),
                "Remove them and try again.",
            ),
            P::TextTooLong => (
                "Text is too long",
                "A changed line can have at most 1,000 characters.".to_string(),
                "Shorten the line.",
            ),
            P::TextOutsideVisibleArea => (
                "Text runs off the page",
                format!("{}, a changed line would run past the edge of the visible page.", on_page(n)),
                "Shorten the line.",
            ),
            P::FaceUnavailable => (
                "That style isn't available",
                on_page_colon(n, &face_sentence(p.face.or(ctx.face), &p.chars)),
                "Keep the current style.",
            ),
            P::StyleUnavailable => (
                "That change isn't available",
                on_page_colon(n, style_sentence(p.field)),
                "Keep the current setting.",
            ),
            P::EditConflict => (
                "Two changes overlap",
                match n {
                    Some(n) => format!("Two text changes on page {n} affect the same line."),
                    None => "Two text changes on this page affect the same line.".to_string(),
                },
                "Undo one of them and try again.",
            ),
            P::TextEditRefused => {
                let reason = p.reason.or(ctx.reason);
                let body = reason.map(|r| r.copy().1).unwrap_or("OffPDF can't change this text safely.");
                (
                    "This text can't be changed safely",
                    on_page_colon(n, body),
                    "Restore the original text for that line.",
                )
            }
            P::Stale => {
                let e = stale(ctx.file_name.unwrap_or("the PDF"));
                return with_problem_details(e, p, n);
            }
            P::PenDrift => (
                "The change would move other text",
                format!("Saving would shift text you didn't change on {}, so nothing was saved.", page_ref(n)),
                "Try a shorter change, or undo it. The original file was not changed.",
            ),
            P::StateChanged => (
                "The change would restyle other content",
                format!("Saving would change how other content on {} is drawn, so nothing was saved.", page_ref(n)),
                "Undo the last change on that page and try again. The original file was not changed.",
            ),
            P::EditVerifyFailed => (
                "The change could not be verified",
                format!("OffPDF checks every change before saving. {}, a changed line couldn't be confirmed to read back exactly as typed with nothing else altered, so nothing was saved.", on_page(n)),
                "Undo that change and try again. The original file was not changed.",
            ),
        };
        let e = AppError::new(self.as_str(), title, message).with_suggestion(suggestion);
        with_problem_details(e, p, n)
    }
}

fn with_problem_details(e: AppError, p: &EditProblem, page: Option<u32>) -> AppError {
    let mut parts = vec![format!("code: {}", p.code.as_str())];
    if let Some(n) = page {
        parts.push(format!("page: {n}"));
    }
    if let Some(r) = p.reason {
        parts.push(format!("reason: {}", r.as_str()));
    }
    if let Some(d) = &p.detail {
        parts.push(d.clone());
    }
    e.with_details(parts.join("; "))
}

fn on_page(n: Option<u32>) -> String {
    match n {
        Some(n) => format!("On page {n}"),
        None => "On this page".to_string(),
    }
}

fn on_page_colon(n: Option<u32>, body: &str) -> String {
    format!("{}: {body}", on_page(n))
}

fn page_ref(n: Option<u32>) -> String {
    match n {
        Some(n) => format!("page {n}"),
        None => "this page".to_string(),
    }
}

fn detail_line(page: Option<u32>, code: &str) -> String {
    match page {
        Some(n) => format!("reason: {code}; page: {n}"),
        None => format!("reason: {code}"),
    }
}

/// Characters for a message: typing order, a space shows as "space".
fn char_list(chars: &[char]) -> String {
    let items: Vec<String> = chars
        .iter()
        .map(|c| {
            if *c == ' ' {
                "space".to_string()
            } else {
                c.to_string()
            }
        })
        .collect();
    items.join(", ")
}

fn face_sentence(face: Option<Face>, chars: &[char]) -> String {
    let face = face.unwrap_or(Face::Regular);
    if chars.is_empty() {
        format!(
            "This page has no {} version of this font.",
            face.adjective()
        )
    } else {
        format!(
            "The {} version of this font can't draw: {}",
            face.adjective(),
            char_list(chars)
        )
    }
}

fn style_sentence(field: Option<StyleField>) -> &'static str {
    match field {
        Some(StyleField::Size) | None => "This line's size is set in a way OffPDF can't change.",
        Some(StyleField::Face) => {
            "This line's font is set in a way OffPDF can't change, so bold and italic aren't available."
        }
        Some(StyleField::Colour) => {
            "This text is drawn with an outline, so its colour can't be changed here."
        }
    }
}

// ---- File-level constructors (copy in §D.11.6) ------------------------------------------

pub fn file_too_large() -> AppError {
    AppError::new(
        "FILE_TOO_LARGE",
        "This PDF is too large to edit text in",
        "Files over 400 MB are not read for text editing.",
    )
    .with_suggestion("Split it with Split PDF and edit the part you need.")
}

pub fn file_too_complex(detail: &str) -> AppError {
    AppError::new(
        "FILE_TOO_COMPLEX",
        "This PDF is too complex to check",
        "Some of its internal data is too large or too deeply nested to check safely.",
    )
    .with_details(detail.to_string())
}

pub fn encrypted() -> AppError {
    AppError::new(
        "ENCRYPTED",
        "This PDF is password-protected",
        "OffPDF can't change text in a protected PDF.",
    )
    .with_suggestion("Remove the password with Unlock PDF, then edit the unlocked copy.")
}

pub fn signed() -> AppError {
    AppError::new(
        "SIGNED",
        "This PDF is digitally signed",
        "Changing its text would break the signature.",
    )
    .with_suggestion("Ask the sender for an unsigned copy if it needs changes.")
}

pub fn unsupported_xfa() -> AppError {
    AppError::new(
        "UNSUPPORTED_XFA",
        "This PDF is a dynamic form",
        "Its pages are generated by the PDF reader, so changes to page text might not show.",
    )
    .with_suggestion("Fill it in a reader that supports dynamic forms.")
}

pub fn malformed_content(detail: &str) -> AppError {
    AppError::new(
        "MALFORMED_CONTENT",
        "Part of this PDF can't be read",
        "OffPDF couldn't read this PDF's structure completely.",
    )
    .with_suggestion("Run it through Repair PDF, then try again.")
    .with_details(detail.to_string())
}

pub fn pdf_needs_repair(warnings: &[String]) -> AppError {
    AppError::new(
        "PDF_NEEDS_REPAIR",
        "This PDF needs repair first",
        "Its internal structure has errors, so OffPDF won't change its text.",
    )
    .with_suggestion("Run it through Repair PDF, then edit the repaired copy.")
    .with_details(warnings.join("\n"))
}

pub fn stale(file_name: &str) -> AppError {
    AppError::new(
        "STALE",
        "The PDF changed on disk",
        format!("\u{201c}{file_name}\u{201d} was changed after you started editing it, so your text changes no longer match it."),
    )
    .with_suggestion("Remove these text changes and make them again. The original file was not changed.")
}

pub fn verifier_missing(tool: &str) -> AppError {
    AppError::new(
        "VERIFIER_MISSING",
        "A checking component is missing",
        "OffPDF needs its bundled Poppler tools to verify text changes.",
    )
    .with_suggestion("Reinstall OffPDF, then try again.")
    .with_details(format!("{tool} could not be started"))
}

pub fn qpdf_too_old(version: &str) -> AppError {
    AppError::new(
        "ENGINE_MISSING",
        "The PDF engine is too old",
        "OffPDF needs a newer version of its bundled qpdf engine to change text.",
    )
    .with_suggestion("Reinstall OffPDF, then try again.")
    .with_details(format!("qpdf 11 or newer is required; found: {version}"))
}

pub fn source_edit_gate_failed(detail: &str) -> AppError {
    AppError::new(
        "SOURCE_EDIT_GATE_FAILED",
        "The edited PDF did not pass the text-change check",
        "The saved file did not contain the text changes exactly as they were checked, so it was not published.",
    )
    .with_suggestion("Try saving again. The original file was not changed.")
    .with_details(detail.to_string())
}

/// `INVALID_PDF` for a file whose cross-reference data can't be read (snapshot preflight).
pub fn invalid_xref(detail: &str) -> AppError {
    AppError::new(
        "INVALID_PDF",
        "The selected file is not a valid PDF",
        "OffPDF could not read this PDF's cross-reference data.",
    )
    .with_suggestion("Make sure the file is a real PDF and is not corrupted.")
    .with_details(detail.to_string())
}

pub const ORIGINAL_UNCHANGED: &str = "The original file was not changed.";

/// Every error leaving the text-edit Save path passes through this (export.rs wraps the results of
/// `prepare_text_sources` and `verify_final`): appends ORIGINAL_UNCHANGED to the suggestion (or sets it)
/// unless the suggestion already ends with it. Open/inspect/preview errors are not wrapped.
pub fn save_failure(e: AppError) -> AppError {
    let suggestion = match e.suggestion.as_deref().map(str::trim_end) {
        None | Some("") => ORIGINAL_UNCHANGED.to_string(),
        Some(s) if s.ends_with(ORIGINAL_UNCHANGED) => s.to_string(),
        Some(s) => format!("{s} {ORIGINAL_UNCHANGED}"),
    };
    AppError {
        suggestion: Some(suggestion),
        ..e
    }
}

// Test-only tables and spellings: production serialises these codes through serde and checks
// reasons one at a time (`cargo check` dead-code gate, review T5 H2).
#[cfg(test)]
impl TextReason {
    /// The 31 run codes in A.10 priority order.
    pub const RUN_PRIORITY: &'static [TextReason] = &[
        R::NestedForm,
        R::SplitContent,
        R::SharedContent,
        R::InlineImage,
        R::InvisibleText,
        R::TextClipMode,
        R::ZeroSize,
        R::Vertical,
        R::MirroredText,
        R::RotatedText,
        R::SkewedText,
        R::Clipped,
        R::OptionalContent,
        R::SoftMask,
        R::Pattern,
        R::ActualText,
        R::MissingFont,
        R::Type3,
        R::FontUnsupported,
        R::FontNotEmbedded,
        R::FontProgramUnsupported,
        R::FontProgramUnreadable,
        R::UnsupportedEncoding,
        R::MissingWidths,
        R::NoTounicode,
        R::AmbiguousUnicode,
        R::RightToLeft,
        R::ComplexScript,
        R::DuplicateText,
        R::PerGlyphText,
        R::NoWritableGlyphs,
    ];
    /// Image occurrences (classifier only), A.10 order.
    pub const IMAGE_PRIORITY: &'static [TextReason] = &[
        R::InlineImage,
        R::NestedForm,
        R::Clipped,
        R::Pattern,
        R::MaskedImage,
        R::SharedXobject,
        R::TransformedImage,
        R::Geometry,
    ];
}

#[cfg(test)]
impl EditProblemCode {
    pub const ALL: &'static [EditProblemCode] = &[
        P::GlyphMissing,
        P::SpaceNotWritable,
        P::InvalidText,
        P::TextTooLong,
        P::TextOutsideVisibleArea,
        P::FaceUnavailable,
        P::StyleUnavailable,
        P::EditConflict,
        P::TextEditRefused,
        P::Stale,
        P::PenDrift,
        P::StateChanged,
        P::EditVerifyFailed,
    ];
}

#[cfg(test)]
impl TextWarningCode {
    pub const ALL: &'static [TextWarningCode] = &[
        TextWarningCode::NextTextOverlap,
        TextWarningCode::EditNotVisible,
        TextWarningCode::PreviewUnavailable,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            TextWarningCode::NextTextOverlap => "NEXT_TEXT_OVERLAP",
            TextWarningCode::EditNotVisible => "EDIT_NOT_VISIBLE",
            TextWarningCode::PreviewUnavailable => "PREVIEW_UNAVAILABLE",
        }
    }
}

#[cfg(test)]
mod tests;
