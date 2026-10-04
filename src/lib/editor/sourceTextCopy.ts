/**
 * Every user-facing sentence of Edit text (SPEC §D.11), in one place.
 *
 * Templates use `{name}` placeholders filled by `fillCopy`. The wording rules
 * (no "cover", "overlay", "flatten", "white-out", "like Word", "we'll") are
 * enforced by `sourceTextCopy.test.ts`, which scans every exported string.
 * Rust builds the Save-time `AppError`s itself (`text_edit/reasons.rs`);
 * `FILE_ERROR_COPY` mirrors those rows for the frontend's own guards.
 */

import type { TextEditProblemCode, TextFace, TextPageReason, TextReason, TextStyleField, TextWarningCode } from "../types";

export interface ReasonCopy {
  short: string;
  title: string;
  body: string;
}

export interface ErrorCopy {
  title: string;
  message: string;
  suggestion: string;
}

/** Replace `{key}` placeholders; unknown keys are left as written. */
export function fillCopy(template: string, vars: Record<string, string | number>): string {
  return template.replace(/\{(\w+)\}/g, (whole, key: string) =>
    Object.prototype.hasOwnProperty.call(vars, key) ? String(vars[key]) : whole,
  );
}

// ---------------------------------------------------------------------------
// §D.11.1 tools, banners, status · §D.11.2 editor and format bar
// ---------------------------------------------------------------------------

export const UI = {
  tool: {
    editText: {
      label: "Edit text",
      title: "Edit text (E): change words already in this PDF, in its own font",
    },
    addText: { label: "Add text", title: "Add text: place a new text box on the page" },
    showOriginal: { label: "Show original", title: "Show the page without your text changes (O)" },
  },
  banner: {
    mode: "Select an outlined line to change its words. Changes use the document's own font, and the rest of the page stays exactly where it is. Dotted lines can't be changed safely — select one to see why.",
    file: "Edit text is off for “{name}”: {message} {suggestion}",
    pageRefused: "Nothing on this page can be changed: {body}",
    noText: "There is no text on this page that OffPDF can change. A scanned page is a picture of text.",
    noneEditable: "None of the text on this page can be changed safely. Select a dotted line to see why.",
    addText: "Adds a new text box. To change words already on the page, use Edit text (E).",
    duplicatePage: "This page is in the list more than once, so its text can't be changed. Remove the extra copy first.",
    stale: "“{name}” changed on disk after you started editing it. The text changes made before that can't be applied.",
    staleAction: "Remove these text changes",
  },
  status: {
    reading: "Reading the text on this page…",
    checking: "Checking the change…",
    showingChanges: "Showing your changes",
    showingOriginal: "Showing the original page",
    previewUnavailable: "Preview unavailable for this page. Your changes are checked again when you save.",
  },
  layer: {
    label: "Text lines on page {n}. Use the arrow keys to move between lines and Enter to edit.",
  },
  run: {
    editable: "Edit text: {text}",
    edited: "Edited text: {new}. Original: {old}",
    refusedDescribe: "Can't be changed: {short}",
  },
  substituted: "This font isn't included in the PDF. Your PDF reader draws it with a similar font.",
  redactionConflict:
    "This page also has text changes. Redaction turns the page into an image, so both can't be saved together.",
  outputAlert:
    "Text changes use the document's own font. Before saving, OffPDF checks that nothing else on those pages moved by more than 0.01 pt. The original file is never changed.",
  list: { label: "Edited text: “{new}” · p{page}" },
  inspector: {
    header: "Text change",
    original: "Original",
    new: "New",
    font: "Font",
    style: "Style",
    note: "The rest of the page stays exactly as it is.",
    editLine: "Edit line",
    restore: "Restore original",
    fontSubset: "{name} · embedded subset",
    fontEmbedded: "{name} · embedded",
    fontNotIncluded: "{name} · not included in the PDF",
  },
  navBlocked: "Finish or cancel the text change on this page first.",
  jobLabel: {
    objects: "Edit PDF · {n} object",
    objectsPlural: "Edit PDF · {n} objects",
    textChanges: " · {m} text change",
    textChangesPlural: " · {m} text changes",
  },
  editor: { aria: "New text for this line" },
  width: {
    wider: "{n} pt wider than before.",
    narrower: "{n} pt narrower than before.",
    same: "Same width as before.",
    aboutPrefix: "About ",
  },
  missing: "This document's font can't draw: {chars}",
  missingSubset: " The file only includes the letters it already uses.",
  spaceName: "space",
  spaceNotWritable: "A space can only go between two letters here, one at a time.",
  invalid: "Line breaks and tabs can't be added. Each box is a single line.",
  tooLong: "A line can have at most 1,000 characters.",
  outside: "The new text would run past the edge of the visible page.",
  overlap: "The new text runs into “{neighbour}”.",
  overlapUnnamed: "The new text runs into the next text on this line.",
  removal: "This line will be removed. Text after it stays where it is.",
  shownLarger: "Shown larger here than it will print.",
  blocked: "Fix or cancel this change first. Press Esc to cancel.",
  announce: { applied: "Change applied. {width}", restored: "Original text restored." },
  notVisible: "This change doesn't change how the page looks. Something may be drawn over this line.",
  bar: {
    label: "Text format",
    smaller: "Smaller",
    larger: "Larger",
    fontSize: "Font size in points",
    bold: "Bold",
    italic: "Italic",
    textColour: "Text colour",
    originalColour: "Original colour",
    black: "Black",
    grey: "Grey",
    red: "Red",
    blue: "Blue",
    green: "Green",
    amber: "Amber",
    customColour: "Custom colour…",
    letterSpacing: "Letter spacing",
    restore: "Restore original",
    cancel: "Cancel",
    done: "Done",
    letters: "Letters",
    lettersTitle: "This font only includes these letters:",
  },
  face: {
    noRegular: "This page has no regular version of this font.",
    noBold: "This page has no bold version of this font.",
    noItalic: "This page has no italic version of this font.",
    noBoldItalic: "This page has no bold italic version of this font.",
    cannotDrawRegular: "The regular version of this font can't draw: {chars}",
    cannotDrawBold: "The bold version of this font can't draw: {chars}",
    cannotDrawItalic: "The italic version of this font can't draw: {chars}",
    cannotDrawBoldItalic: "The bold italic version of this font can't draw: {chars}",
  },
  style: {
    size: "This line's size is set in a way OffPDF can't change.",
    face: "This line's font is set in a way OffPDF can't change, so bold and italic aren't available.",
    colour: "This text is drawn with an outline, so its colour can't be changed here.",
  },
  faceLabel: { regular: "Regular", bold: "Bold", italic: "Italic", boldItalic: "Bold italic" },
  popover: {
    title: "This text can't be changed safely",
    footer: "You can add a new text box on top with Add text. It won't replace this text.",
    addTextHere: "Add text here",
    close: "Close",
  },
} as const;

// ---------------------------------------------------------------------------
// §D.11.3 reasons
// ---------------------------------------------------------------------------

export const REASON_COPY: Record<TextReason, ReasonCopy> = {
  NESTED_FORM: { short: "part of a reused block", title: "Part of a reused block", body: "This text is inside a block the document can reuse, so changing it here could change it in other places too." },
  SPLIT_CONTENT: { short: "stored in two pieces", title: "Stored in two pieces", body: "The instructions that draw this line are split across two parts of the page." },
  SHARED_CONTENT: { short: "shared with other pages", title: "Shared with other pages", body: "This part of the page is shared with other pages, so a change here would change them too." },
  INLINE_IMAGE: { short: "after an unreadable picture", title: "After an unreadable picture", body: "A picture stored inside this page can't be measured exactly, so text drawn after it can't be changed safely." },
  INVISIBLE_TEXT: { short: "hidden text", title: "Hidden text", body: "This is hidden text, such as the searchable layer of a scan. Changing it would change nothing you can see." },
  TEXT_CLIP_MODE: { short: "used as a shape", title: "Used as a shape", body: "This text is used as a shape that clips other content, so it can't be changed safely." },
  ZERO_SIZE: { short: "no visible size", title: "No visible size", body: "This text is drawn at zero size." },
  VERTICAL: { short: "vertical text", title: "Vertical text", body: "This text runs top to bottom. Only lines that read across the page can be changed." },
  MIRRORED_TEXT: { short: "mirrored", title: "Mirrored text", body: "This text is drawn mirrored, so new letters can't be placed the same way." },
  ROTATED_TEXT: { short: "turned at an angle", title: "Turned at an angle", body: "This line doesn't run straight across the page as shown. Only level lines can be changed." },
  SKEWED_TEXT: { short: "tilted line", title: "Tilted line", body: "This line's baseline is tilted by the page layout, so it can't be changed safely." },
  CLIPPED: { short: "partly cut off", title: "Partly cut off", body: "Part of this text is cut off by the page edge or a clipping area, so a change might not show." },
  OPTIONAL_CONTENT: { short: "on a layer", title: "On a layer", body: "This text is on a layer that is hidden or depends on viewer settings, so a change might not show." },
  SOFT_MASK: { short: "drawn through a mask", title: "Masked text", body: "This text is drawn through a transparency mask, so a change could look different from what you type." },
  PATTERN: { short: "pattern fill", title: "Pattern fill", body: "This text is filled with a pattern or gradient rather than a colour." },
  ACTUAL_TEXT: { short: "separate reading copy", title: "Separate reading copy", body: "The document keeps a separate copy of this text for search and screen readers. Changing only the visible letters would make the two disagree." },
  MISSING_FONT: { short: "font missing", title: "Font missing", body: "The font this text uses is missing from the document." },
  TYPE3: { short: "font made of drawings", title: "Font made of drawings", body: "This text uses a font drawn from shapes, which OffPDF can't type with." },
  FONT_UNSUPPORTED: { short: "unusual font setup", title: "Unusual font setup", body: "This font is set up in a way OffPDF can't change safely." },
  FONT_NOT_EMBEDDED: { short: "font not included", title: "Font not included", body: "This font isn't included in the PDF, so OffPDF can't check which letters it can draw." },
  FONT_PROGRAM_UNSUPPORTED: { short: "font type not supported yet", title: "Font type not supported yet", body: "This text uses a kind of embedded font that OffPDF can't check yet." },
  FONT_PROGRAM_UNREADABLE: { short: "font can't be read", title: "Font can't be read", body: "The font included in this PDF can't be read, so OffPDF can't check which letters it can draw." },
  UNSUPPORTED_ENCODING: { short: "unsupported letter mapping", title: "Letter mapping not supported", body: "This font maps letters in a way OffPDF can't write yet." },
  MISSING_WIDTHS: { short: "letter widths missing", title: "Letter widths missing", body: "The document doesn't say how wide this font's letters are, so the rest of the line couldn't be kept in place." },
  NO_TOUNICODE: { short: "letters not identified", title: "Letters not identified", body: "The document doesn't say which letters this font draws, so OffPDF can't read or retype them." },
  AMBIGUOUS_UNICODE: { short: "letters can't be confirmed", title: "Letters can't be confirmed", body: "Some letters in this line can't be read with certainty, so OffPDF can't show the current text reliably." },
  RIGHT_TO_LEFT: { short: "right-to-left script", title: "Right-to-left script", body: "The document has already ordered and shaped these letters, and changing them would undo that work." },
  COMPLEX_SCRIPT: { short: "shaped script", title: "Shaped script", body: "This script joins or reorders letters, and the document has already done that shaping. OffPDF can't redo it yet." },
  DUPLICATE_TEXT: { short: "drawn twice", title: "Drawn twice", body: "This text is drawn twice (for example as a shadow or to look bolder), so changing one copy would leave the other behind." },
  PER_GLYPH_TEXT: { short: "letters placed one by one", title: "Letters placed one by one", body: "This page places every letter on its own, so a line can't be changed as one piece." },
  NO_WRITABLE_GLYPHS: { short: "no letters to type with", title: "No letters to type with", body: "The font this line uses has no letters OffPDF can confirm, so nothing can be typed with it." },
};

export const PAGE_REASON_COPY: Record<TextPageReason, ReasonCopy> = {
  MALFORMED_CONTENT: { short: "page can't be read", title: "Damaged page content", body: "OffPDF couldn't read this page's content completely, so none of its text can be changed." },
  UNSUPPORTED_FILTER: { short: "unusual compression", title: "Unusual compression", body: "This page's content is stored or compressed in a way OffPDF can't check." },
  PAGE_TOO_COMPLEX: { short: "too much content", title: "Too complex to check", body: "This page has too much content to check safely." },
  GEOMETRY: { short: "custom page unit", title: "Custom page unit", body: "This page uses a custom unit size or unreadable page boxes, which OffPDF doesn't edit yet." },
};

/** The "(unknown)" row: any code without its own copy. */
export const GENERIC_REASON_COPY: ReasonCopy = {
  short: "can't be changed safely",
  title: "Can't be changed safely",
  body: "OffPDF can't change this text safely.",
};

/** Copy for a run or page reason; unknown or missing codes get the generic row. */
export function reasonCopy(code: string | null | undefined): ReasonCopy {
  if (code && Object.prototype.hasOwnProperty.call(REASON_COPY, code)) return REASON_COPY[code as TextReason];
  if (code && Object.prototype.hasOwnProperty.call(PAGE_REASON_COPY, code)) {
    return PAGE_REASON_COPY[code as TextPageReason];
  }
  return GENERIC_REASON_COPY;
}

// ---------------------------------------------------------------------------
// §D.11.4 problems · §D.11.5 warnings
// ---------------------------------------------------------------------------

/** Inline message per problem. Context-dependent codes hold their default
 * (the same defaults Rust uses: face → regular, field → size, reason → generic). */
export const PROBLEM_COPY: Record<TextEditProblemCode, string> = {
  GLYPH_MISSING: UI.missing,
  SPACE_NOT_WRITABLE: UI.spaceNotWritable,
  INVALID_TEXT: UI.invalid,
  TEXT_TOO_LONG: UI.tooLong,
  TEXT_OUTSIDE_VISIBLE_AREA: UI.outside,
  FACE_UNAVAILABLE: UI.face.noRegular,
  STYLE_UNAVAILABLE: UI.style.size,
  EDIT_CONFLICT: "This line is already part of another change on this page.",
  TEXT_EDIT_REFUSED: GENERIC_REASON_COPY.body,
  STALE: UI.banner.stale,
  PEN_DRIFT: "That change would move other text on this page, so it can't be saved. Undo it or try a shorter change.",
  STATE_CHANGED: "That change would alter how other content on this page is drawn, so it can't be saved.",
  EDIT_VERIFY_FAILED:
    "OffPDF couldn't confirm this change reads back exactly as typed with nothing else altered, so it can't be saved.",
};

export const WARNING_COPY: Record<TextWarningCode, string> = {
  NEXT_TEXT_OVERLAP: UI.overlap,
  EDIT_NOT_VISIBLE: UI.notVisible,
  PREVIEW_UNAVAILABLE: UI.status.previewUnavailable,
};

const FACE_NO: Record<TextFace, string> = {
  regular: UI.face.noRegular,
  bold: UI.face.noBold,
  italic: UI.face.noItalic,
  boldItalic: UI.face.noBoldItalic,
};

const FACE_CANNOT_DRAW: Record<TextFace, string> = {
  regular: UI.face.cannotDrawRegular,
  bold: UI.face.cannotDrawBold,
  italic: UI.face.cannotDrawItalic,
  boldItalic: UI.face.cannotDrawBoldItalic,
};

/** Characters for a message, in typing order; a space shows as "space". */
export function charList(chars: readonly string[]): string {
  return chars.map((c) => (c === " " ? UI.spaceName : c)).join(", ");
}

export interface ProblemCopyCtx {
  face?: TextFace | null;
  field?: TextStyleField | null;
  reason?: string | null;
  /** The run's font is an embedded subset (GLYPH_MISSING appends a note). */
  subset?: boolean;
  /** File name for STALE. */
  name?: string | null;
}

/** The inline message for a problem code (§D.11.4). */
export function problemCopy(code: TextEditProblemCode, chars: readonly string[] = [], ctx: ProblemCopyCtx = {}): string {
  switch (code) {
    case "GLYPH_MISSING":
      return fillCopy(UI.missing, { chars: charList(chars) }) + (ctx.subset ? UI.missingSubset : "");
    case "FACE_UNAVAILABLE": {
      const face = ctx.face ?? "regular";
      return chars.length > 0 ? fillCopy(FACE_CANNOT_DRAW[face], { chars: charList(chars) }) : FACE_NO[face];
    }
    case "STYLE_UNAVAILABLE":
      return UI.style[ctx.field ?? "size"];
    case "TEXT_EDIT_REFUSED":
      return reasonCopy(ctx.reason).body;
    case "STALE":
      return fillCopy(UI.banner.stale, { name: ctx.name ?? "the PDF" });
    default:
      return PROBLEM_COPY[code];
  }
}

/** Shorten to `max` characters (code points), ending with "…" when cut. */
export function truncateCopy(text: string, max: number): string {
  const chars = Array.from(text);
  if (chars.length <= max) return text;
  return `${chars.slice(0, Math.max(0, max - 1)).join("")}…`;
}

const NEIGHBOUR_MAX_CHARS = 24;

/** The message for a warning; `neighbour` is the text the new line runs into. */
export function warningCopy(code: TextWarningCode, neighbour?: string | null): string {
  if (code !== "NEXT_TEXT_OVERLAP") return WARNING_COPY[code];
  const trimmed = neighbour?.trim();
  if (!trimmed) return UI.overlapUnnamed;
  return fillCopy(UI.overlap, { neighbour: truncateCopy(trimmed, NEIGHBOUR_MAX_CHARS) });
}

function formatPt(n: number, decimals: number): string {
  return String(Number(n.toFixed(decimals)));
}

/** "3.2 pt wider than before." — estimates (`exact` false) start with "About ". */
export function widthSentence(deltaPt: number, exact: boolean): string {
  const decimals = exact ? 2 : 1;
  const magnitude = Number.isFinite(deltaPt) ? Number(Math.abs(deltaPt).toFixed(decimals)) : 0;
  if (magnitude === 0) return UI.width.same;
  const sentence = fillCopy(deltaPt > 0 ? UI.width.wider : UI.width.narrower, { n: formatPt(magnitude, decimals) });
  return exact ? sentence : UI.width.aboutPrefix + sentence;
}

// ---------------------------------------------------------------------------
// Small composed strings
// ---------------------------------------------------------------------------

/** Text that already ends a sentence (closing quotes or brackets may follow the mark). */
const ENDS_SENTENCE = /[.!?…]["'”’»)\]]*$/u;

/**
 * Label of an edited line: `Edited text: {new}. Original: {old}`. The template's full stop
 * closes `{new}`; when the new text already ends with its own (“…within 30 days.”), the
 * template's is dropped instead of doubling it (“days..”).
 */
export function editedRunLabel(newText: string, oldText: string): string {
  const template = ENDS_SENTENCE.test(newText.trimEnd()) ? UI.run.edited.replace("{new}.", "{new}") : UI.run.edited;
  return fillCopy(template, { new: newText, old: oldText });
}

const LIST_LABEL_MAX_CHARS = 28;

/** Object list label: `Edited text: “…” · p3` (new text cut at 28 characters + "…"). */
export function listLabel(newText: string, pageNumber: number): string {
  const chars = Array.from(newText);
  const shown = chars.length > LIST_LABEL_MAX_CHARS ? `${chars.slice(0, LIST_LABEL_MAX_CHARS).join("")}…` : newText;
  return fillCopy(UI.list.label, { new: shown, page: pageNumber });
}

/** `Edit PDF · 3 objects · 2 text changes` (the text part only when m > 0). */
export function jobLabel(objectCount: number, textChangeCount: number): string {
  const head = fillCopy(objectCount === 1 ? UI.jobLabel.objects : UI.jobLabel.objectsPlural, { n: objectCount });
  if (textChangeCount <= 0) return head;
  const tail = fillCopy(textChangeCount === 1 ? UI.jobLabel.textChanges : UI.jobLabel.textChangesPlural, {
    m: textChangeCount,
  });
  return head + tail;
}

/** `banner.file` for a source that could not be opened. */
export function fileBanner(name: string, error: { message: string; suggestion?: string | null }): string {
  return fillCopy(UI.banner.file, { name, message: error.message, suggestion: error.suggestion ?? "" }).trimEnd();
}

/** Inspector font row: `Calibri · embedded subset` / `· embedded` / `· not included in the PDF`. */
export function fontDescription(font: { displayName: string; embedded: boolean; subset: boolean }): string {
  const template = !font.embedded
    ? UI.inspector.fontNotIncluded
    : font.subset
      ? UI.inspector.fontSubset
      : UI.inspector.fontEmbedded;
  return fillCopy(template, { name: font.displayName });
}

// ---------------------------------------------------------------------------
// §D.11.6 AppError copy (Rust builds these; mirrored for frontend guards)
// ---------------------------------------------------------------------------

export const FILE_ERROR_COPY: Record<string, ErrorCopy> = {
  ENCRYPTED: { title: "This PDF is password-protected", message: "OffPDF can't change text in a protected PDF.", suggestion: "Remove the password with Unlock PDF, then edit the unlocked copy." },
  SIGNED: { title: "This PDF is digitally signed", message: "Changing its text would break the signature.", suggestion: "Ask the sender for an unsigned copy if it needs changes." },
  UNSUPPORTED_XFA: { title: "This PDF is a dynamic form", message: "Its pages are generated by the PDF reader, so changes to page text might not show.", suggestion: "Fill it in a reader that supports dynamic forms." },
  PDF_NEEDS_REPAIR: { title: "This PDF needs repair first", message: "Its internal structure has errors, so OffPDF won't change its text.", suggestion: "Run it through Repair PDF, then edit the repaired copy." },
  FILE_TOO_LARGE: { title: "This PDF is too large to edit text in", message: "Files over 400 MB are not read for text editing.", suggestion: "Split it with Split PDF and edit the part you need." },
  FILE_TOO_COMPLEX: { title: "This PDF is too complex to check", message: "Some of its internal data is too large or too deeply nested to check safely.", suggestion: "" },
  MALFORMED_CONTENT: { title: "Part of this PDF can't be read", message: "OffPDF couldn't read this PDF's structure completely.", suggestion: "Run it through Repair PDF, then try again." },
  STALE: { title: "The PDF changed on disk", message: "“{name}” was changed after you started editing it, so your text changes no longer match it.", suggestion: "Remove these text changes and make them again. The original file was not changed." },
  VERIFIER_MISSING: { title: "A checking component is missing", message: "OffPDF needs its bundled Poppler tools to verify text changes.", suggestion: "Reinstall OffPDF, then try again." },
  GLYPH_MISSING: { title: "The font can't draw some letters", message: "On page {n}, the document's font can't draw: {chars}", suggestion: "Use other characters, or add a new text box with Add text." },
  SPACE_NOT_WRITABLE: { title: "A space can't go there", message: "On page {n}, spaces in the changed line are gaps between letters, so a space can only go between two letters, one at a time.", suggestion: "Remove the extra space." },
  INVALID_TEXT: { title: "These characters can't be added", message: "Line breaks, tabs and control characters can't be added. Each box is a single line.", suggestion: "Remove them and try again." },
  TEXT_TOO_LONG: { title: "Text is too long", message: "A changed line can have at most 1,000 characters.", suggestion: "Shorten the line." },
  TEXT_OUTSIDE_VISIBLE_AREA: { title: "Text runs off the page", message: "On page {n}, a changed line would run past the edge of the visible page.", suggestion: "Shorten the line." },
  FACE_UNAVAILABLE: { title: "That style isn't available", message: "On page {n}: {sentence}", suggestion: "Keep the current style." },
  STYLE_UNAVAILABLE: { title: "That change isn't available", message: "On page {n}: {sentence}", suggestion: "Keep the current setting." },
  EDIT_CONFLICT: { title: "Two changes overlap", message: "Two text changes on page {n} affect the same line.", suggestion: "Undo one of them and try again." },
  TEXT_EDIT_REFUSED: { title: "This text can't be changed safely", message: "On page {n}: {body}", suggestion: "Restore the original text for that line." },
  PEN_DRIFT: { title: "The change would move other text", message: "Saving would shift text you didn't change on page {n}, so nothing was saved.", suggestion: "Try a shorter change, or undo it. The original file was not changed." },
  STATE_CHANGED: { title: "The change would restyle other content", message: "Saving would change how other content on page {n} is drawn, so nothing was saved.", suggestion: "Undo the last change on that page and try again. The original file was not changed." },
  EDIT_VERIFY_FAILED: { title: "The change could not be verified", message: "OffPDF checks every change before saving. On page {n}, a changed line couldn't be confirmed to read back exactly as typed with nothing else altered, so nothing was saved.", suggestion: "Undo that change and try again. The original file was not changed." },
  SOURCE_EDIT_GATE_FAILED: { title: "The edited PDF did not pass the text-change check", message: "The saved file did not contain the text changes exactly as they were checked, so it was not published.", suggestion: "Try saving again. The original file was not changed." },
  TEXT_EDIT_ON_REDACTED_PAGE: { title: "Redaction and text change on the same page", message: "Page {n} has both a redaction and a text change. Redaction turns the page into an image, so the text change would be lost.", suggestion: "Remove the redaction or the text change on that page." },
  TEXT_EDIT_DUPLICATE_PAGE: { title: "This page appears twice", message: "Page {p} of “{name}” is in the list more than once and has a text change.", suggestion: "Remove the extra copy of the page, then save again." },
  TOO_MANY_TEXT_EDITS: { title: "Too many text changes", message: "This save has more than 500 changed lines.", suggestion: "Save in smaller batches." },
  // Pre-existing Rust errors that the text-edit commands can also return (not §D.11.6 rows).
  INVALID_PDF: { title: "The selected file is not a valid PDF", message: "OffPDF could not open this file as a PDF document.", suggestion: "Make sure the file is a real PDF and is not corrupted." },
  INVALID_PAGES: { title: "Invalid page selection", message: "This page is not in the PDF.", suggestion: "Open the page again from the page list." },
  ENGINE_MISSING: { title: "PDF engine not found", message: "The bundled qpdf engine could not be located.", suggestion: "Reinstall OffPDF, then try again." },
  BAD_EDIT: { title: "Could not save", message: "A text change has a setting OffPDF can't use.", suggestion: "Undo the last change and try again." },
};

/** Save-guard toasts that exist only in the frontend (§D.11.6, last paragraph). */
export const SAVE_GUARD_COPY = {
  notReadyTitle: "Text changes can't be saved yet",
  notReadyDescription: "Open Edit text on “{name}” again, then save.",
} as const;
