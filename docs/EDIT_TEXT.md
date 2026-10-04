# Edit text

Edit text changes words that are already in a PDF, in the document's own font,
without moving anything else on the page. It is part of **Edit PDF** (tool
**Edit text**, shortcut **E**).

This page is the user guide (what Edit text can and cannot change, and why:
sections 2–7), the decision record for issue #11 (section 1, evidence in 11–13)
and the contributor reference for the engine (sections 8–10 and 16). The editor's
data model (`kind: "sourceText"` objects, keyboard contract) is in
[`src/lib/editor/EDIT_MODEL.md`](../src/lib/editor/EDIT_MODEL.md).

## 1. Decision (#11)

| | |
| --- | --- |
| Question | Can OffPDF reliably change existing page text or embedded images without pretending that PDF is a word-processing format? |
| Decision | **Yes, for a bounded, fail-closed subset of text.** Existing images are not replaced in v0.4. |
| Status | Accepted with the v0.4 implementation. The maintainer confirms it on #11 before the release PR is merged. |
| Date | 2026-10-02 |
| Owner | @McanKul (maintainer) |
| Scope | One line at a time, inside the page's own content stream, in a font the page already uses, verified before it is saved. |
| Not in scope | Image replacement (the image half of #11), text inside reusable blocks (Form XObjects), annotation text, shared content streams, Type3, vertical, right-to-left and complex scripts, reflow, new fonts. See [section 15](#15-known-limitations-and-v05-candidates). |
| Engine | lopdf 0.34 (read only) + OffPDF's own bounded lexer and walker, qpdf as the only writer, Poppler as an independent verifier. No PDFium sidecar ([section 13](#13-engine-decision)). |
| Evidence | The reason codes ([6](#6-reason-codes)), the compatibility matrix ([11](#11-compatibility-matrix)), the measured performance and engine matrix ([12](#12-performance-and-engine-matrix)) and the adversarial gate tests ([9](#9-verification)). |

The exit criterion of #11 is met by construction: every line Edit text cannot
change safely is refused with a named reason, and no refusal falls back to
painting new text on top of old text.

## 2. What Edit text does and never does

**It does:**

- change the words of one line, shorter, longer or empty (an empty line is
  removed, and the text after it stays where it was);
- keep the unchanged letters of the line exactly as they were (same glyph codes
  and the same kerning between them);
- keep everything else on the page where it was: every other glyph is checked to
  stay within 0.01 pt;
- offer size, bold or italic (when the page already has that face), fill colour
  and letter spacing for the whole line;
- show the real result: after each change it renders the page that Save would
  write, through the same checks as Save;
- explain every line it refuses: select a dotted line to see the reason;
- write a new file. The original is never overwritten.

**It never:**

- covers old text with a white box and new text, adds an overlay, or adds an
  annotation in place of the old words;
- flattens or rasterises a page, or re-saves a page through a PDF library;
- substitutes or embeds a font, or draws a letter the embedded font does not
  contain;
- reflows a paragraph, wraps a line, or moves other lines;
- silently skips a change: a change that cannot be proven is refused, and Save
  publishes nothing.

What you see while editing: dashed outline = editable, dotted = can't be changed
(text inside reusable blocks included), green outline with a dot = edited. The
line editor shows your draft in a similar serif, sans or monospace system font
(from the font's flags, else its name); after **Done** the preview shows the
document's own font. **Show original** (**O**) compares the original page with
that preview while one is on screen. **Save** with a line still open finishes it
first; a draft that can't be applied stays open and nothing is saved. **Add
text** places a new text box and never replaces existing text; **Add text here**
in the reason dialog puts one over the line (an upright one-line box at the
line's start when the line is not level on screen).

Before you rely on it, read the limits in [section 15](#15-known-limitations-and-v05-candidates).
The two you are most likely to meet: a subset font only contains the letters the
document already uses, and a file with any damaged JPEG image is refused with
`PDF_NEEDS_REPAIR`.

## 3. Supported subset

### Unit of editing

| Rule | v0.4 |
| --- | --- |
| What changes | The text of one line: one show operator (`Tj`, `TJ`, `'`, `"`), or several consecutive ones joined into one visual line. Only text the page's own `/Contents` draws directly; text drawn through a Form XObject is listed as refused lines (`NESTED_FORM`) after the page's own lines. |
| Joining | Same baseline (0.01 pt) and direction, same graphics and text state except the font, the same marked-content context, the same font or a sibling face of it, a gap within ±0.3 em, nothing painted in between, at most 256 pieces. Gaps between joined pieces become `TJ` kerns, so unchanged glyphs keep their positions. |
| Per-glyph pages | After joining, if a page has at least 12 editable lines and 80 % or more of them are one character long, those one-character lines are refused (`PER_GLYPH_TEXT`). |
| Lines | Single line. No reflow, wrapping or multi-line editing. Lines that are only spaces are not offered. |
| Position | The new text starts where the old text started. The pen after the line ends where it ended before (±0.01 pt). |
| No-op | Text equal to the current text with no style change writes nothing. |

### Fonts

| Font setup | v0.4 | How a letter is proven drawable |
| --- | --- | --- |
| Simple TrueType, embedded (full or subset) | Editable | The glyph is found by every applicable lookup (glyph name, Unicode cmap, MacRoman cmap, symbolic cmap) and all lookups agree; the outline has at least one segment |
| Simple CFF (`/Type1C`) and OpenType (`/OpenType`) | Editable | Glyph by name from the PDF encoding, or by the program's own built-in encoding (read by OffPDF's bounded parser); outline present |
| Simple Type1 (`/FontFile`) | Editable | eexec decrypted; the charstring draws at least one path; the name is also in `/CharSet` when that exists |
| Standard 14 (Helvetica, Times, Courier and common aliases such as Arial, Times New Roman, Courier New) | Editable | Glyph name in that face's AFM; widths from `/Widths` or that face's AFM |
| Simple, not embedded, with `/Widths` | Editable, marked "not included in the PDF" | Width > 0 and a Latin, Greek or Cyrillic character; your PDF reader substitutes the font exactly as it already does for the existing text |
| Type0 `/Identity-H` with CIDFontType2 (TrueType) | Editable | CID → GID through `/CIDToGIDMap`; outline present; ToUnicode required |
| Type0 `/Identity-H` with CIDFontType0 (CFF) | Editable | CID → GID through the CFF charset; outline present |
| Font set by an ExtGState `/Font` | Text, colour and letter spacing; size and bold/italic unavailable | as its class |
| Type3, vertical, predefined non-Identity CMaps, Symbol, ZapfDingbats, MMType1, CFF2-only programs, sfnt programs with both `glyf` and CFF outlines | Refused | see [section 6](#6-reason-codes) |

A subset font only contains the letters the document already uses. Edit text
shows the available letters (**Letters** in the format bar) and names any
letter it cannot draw before you commit.

### Characters

- Reading: Latin (Basic to Extended-B and Extended Additional), IPA, Greek,
  Cyrillic, general punctuation, symbols, currency, arrows, maths operators,
  box drawing, CJK punctuation, Hiragana, Katakana, CJK ideographs, Hangul
  syllables and full-width forms. Ligatures such as "ﬁ" read as "fi" but are
  never typed.
- Refused: right-to-left scripts (`RIGHT_TO_LEFT`) and scripts that need shaping
  (`COMPLEX_SCRIPT`).
- Typing: what you type is normalised (NFC). Line breaks, tabs and control
  characters are rejected (`INVALID_TEXT`). At most 1,000 characters per line
  (`TEXT_TOO_LONG`). A character the font cannot draw is named
  (`GLYPH_MISSING`). Only characters in the Basic Multilingual Plane.
- Spaces: when the font has a space glyph, a typed space is a real space.
  Otherwise (typical for pdfTeX) word gaps are kerns; a typed space then becomes
  a kern equal to the line's usual word gap, and it must sit between two
  letters, one at a time (`SPACE_NOT_WRITABLE`).

### Content and geometry

- Content filters: none, Flate (no predictor), ASCIIHex and ASCII85, in chains
  of up to 4. Anything else refuses the page (`UNSUPPORTED_FILTER`).
- `/Contents` may be one stream or an array of parts. A show operator that crosses
  two parts is refused (`SPLIT_CONTENT`). Viewers read a part boundary differently
  (nothing, a space or a newline), so parts that meet inside a token, comment or
  `BX`…`EX` section, run two tokens together, or follow an inline image ending
  within 64 bytes of the part's end refuse the page (`MALFORMED_CONTENT`); parts
  meeting between operators (`ET`|`BT`), after a number (`0`|`cm`) or at a comment's
  end of line are fine.
- Inline images are measured exactly when their length can be proven; text drawn
  after an inline image whose end is only guessed is refused (`INLINE_IMAGE`).
- `/Rotate` 0, 90, 180 and 270 and offset CropBoxes are supported. Orientation
  is judged as displayed: text that reads level on screen is editable, including
  landscape pages made by rotation and synthetic italic (shear up to 0.5).
- `/UserUnit` other than 1, unreadable page boxes and non-integer rotations
  refuse the page (`GEOMETRY`).
- Text render modes 0, 1 and 2 are editable (colour only for mode 0).
- Visible optional-content layers are editable; hidden or conditional ones are
  refused (`OPTIONAL_CONTENT`).
- Encrypted, signed (an applied signature) and dynamic XFA files are refused when
  they are opened. An unedited signed file may still be appended in the same Save.

## 4. Style controls and units

| Control | Range and units | Written as | Unavailable when |
| --- | --- | --- | --- |
| Size | Effective points as you see them, 4–144, steps of 0.5 | `Tf` scaled so the size on the page equals the target; the original `Tf` is restored byte for byte after the line | the font comes from an ExtGState (`STYLE_UNAVAILABLE`) |
| Bold / Italic | Toggles | Switch to the sibling face already in the page's font resources; the whole line is re-encoded in that face | no such face on the page, or it cannot draw the text (`FACE_UNAVAILABLE`); ExtGState font (`STYLE_UNAVAILABLE`) |
| Colour (fill) | Original colour, six inks (black, grey, red, blue, green, amber), or a custom `#rrggbb` | `r g b rg`; the colour in force after the line is restored byte for byte | text drawn with an outline, render mode ≠ 0 (`STYLE_UNAVAILABLE`) |
| Letter spacing | Effective points, −2 to +10, steps of 0.1 | `Tc`; the original `Tc` is restored byte for byte | — |

Word spacing, horizontal scaling, rise, stroke colour, render mode, underline,
alignment and changing the font family are not offered. A style value within
0.001 of the current one is treated as unchanged. Style applies to the whole line.

## 5. Width policy

- Shorter or equal text is always allowed.
- Longer text is allowed while it stays inside the visible page and its clip
  area, or within the line's original extent if that was larger. Past that
  edge the change is blocked (`TEXT_OUTSIDE_VISIBLE_AREA`).
- Running into the next text on the same line (a longer table cell, say) is a
  warning (`NEXT_TEXT_OVERLAP`), shown in the editor, in the inspector and in
  the Save job message; the change still saves. The preview shows the real
  overlap.
- Text after the changed part of the line keeps its position. When the line
  contains a column gap of at least 1 em, the gap absorbs the width change and
  the columns after it do not move. Otherwise one kern at the end of the line
  restores the pen.

## 6. Reason codes

Every line that cannot be changed carries exactly one code. When several apply,
the first in this table wins (all of them are kept in the technical details).
**Run** codes refuse one line. **Page** codes refuse every line on the page, and
the page shows one banner instead of outlines.

<!-- ui-copy:start -->
| Code | Level | When | Short | Title | What OffPDF says |
| --- | --- | --- | --- | --- | --- |
| `NESTED_FORM` | run | Drawn from inside a Form XObject (a reusable block), at any depth; listed after the page's own lines | part of a reused block | Part of a reused block | This text is inside a block the document can reuse, so changing it here could change it in other places too. |
| `SPLIT_CONTENT` | run | The show operator's bytes cross the boundary between two `/Contents` parts | stored in two pieces | Stored in two pieces | The instructions that draw this line are split across two parts of the page. |
| `SHARED_CONTENT` | run | The content part is reachable from more than one page or object (page listed twice in the page tree, part stream or `/Contents` array referenced twice) | shared with other pages | Shared with other pages | This part of the page is shared with other pages, so a change here would change them too. |
| `INLINE_IMAGE` | run | Drawn after an inline image whose end could not be proven | after an unreadable picture | After an unreadable picture | A picture stored inside this page can't be measured exactly, so text drawn after it can't be changed safely. |
| `INVISIBLE_TEXT` | run | Text render mode 3 (invisible, e.g. the searchable layer of a scan) | hidden text | Hidden text | This is hidden text, such as the searchable layer of a scan. Changing it would change nothing you can see. |
| `TEXT_CLIP_MODE` | run | Text render modes 4–7 (text used as a clipping path) | used as a shape | Used as a shape | This text is used as a shape that clips other content, so it can't be changed safely. |
| `ZERO_SIZE` | run | `Tf 0`, `Tz 0` or a singular text matrix | no visible size | No visible size | This text is drawn at zero size. |
| `VERTICAL` | run | Vertical writing (`Identity-V`, a `-V` CMap, `/WMode 1`) | vertical text | Vertical text | This text runs top to bottom. Only lines that read across the page can be changed. |
| `MIRRORED_TEXT` | run | Exactly one axis flipped as displayed (negative `Tz`, mirrored `Tm`) | mirrored | Mirrored text | This text is drawn mirrored, so new letters can't be placed the same way. |
| `ROTATED_TEXT` | run | Not level as displayed: upside down or turned at an angle | turned at an angle | Turned at an angle | This line doesn't run straight across the page as shown. Only level lines can be changed. |
| `SKEWED_TEXT` | run | Neither level, mirrored nor a pure rotation as displayed: a tilted baseline or a slant beyond 0.5 | tilted line | Tilted line | This line's baseline is tilted by the page layout, so it can't be changed safely. |
| `CLIPPED` | run | Ink box outside the visible page box (CropBox ∩ MediaBox), or cut by a clip that is not a set of rectangles containing it | partly cut off | Partly cut off | Part of this text is cut off by the page edge or a clipping area, so a change might not show. |
| `OPTIONAL_CONTENT` | run | Inside optional content that is hidden by default, a membership dictionary or an unresolvable group | on a layer | On a layer | This text is on a layer that is hidden or depends on viewer settings, so a change might not show. |
| `SOFT_MASK` | run | An ExtGState soft mask is in force | drawn through a mask | Masked text | This text is drawn through a transparency mask, so a change could look different from what you type. |
| `PATTERN` | run | Fill (or stroke, for modes 1–2) uses a Pattern colour space | pattern fill | Pattern fill | This text is filled with a pattern or gradient rather than a colour. |
| `ACTUAL_TEXT` | run | Marked content or its structure element carries `/ActualText` | separate reading copy | Separate reading copy | The document keeps a separate copy of this text for search and screen readers. Changing only the visible letters would make the two disagree. |
| `MISSING_FONT` | run | The font name is not in the page's resources | font missing | Font missing | The font this text uses is missing from the document. |
| `TYPE3` | run | Type3 font | font made of drawings | Font made of drawings | This text uses a font drawn from shapes, which OffPDF can't type with. |
| `FONT_UNSUPPORTED` | run | Malformed `/Widths`, `/FirstChar`, `/LastChar`, `/W` or `/Differences`; MMType1 | unusual font setup | Unusual font setup | This font is set up in a way OffPDF can't change safely. |
| `FONT_NOT_EMBEDDED` | run | Type0 (CID) font without an embedded program | font not included | Font not included | This font isn't included in the PDF, so OffPDF can't check which letters it can draw. |
| `FONT_PROGRAM_UNSUPPORTED` | run | Embedded program of an unknown or mismatched `FontFile3` subtype, a CFF2-only program, or an sfnt holding both `glyf` and `CFF `/`CFF2` outlines | font type not supported yet | Font type not supported yet | This text uses a kind of embedded font that OffPDF can't check yet. |
| `FONT_PROGRAM_UNREADABLE` | run | The embedded program does not parse | font can't be read | Font can't be read | The font included in this PDF can't be read, so OffPDF can't check which letters it can draw. |
| `UNSUPPORTED_ENCODING` | run | Non-Identity CMaps, `/MacExpertEncoding`, Symbol and ZapfDingbats, symbolic fonts that are not embedded | unsupported letter mapping | Letter mapping not supported | This font maps letters in a way OffPDF can't write yet. |
| `MISSING_WIDTHS` | run | Non-Standard-14 simple font without `/Widths`, or text drawn after an advance OffPDF cannot know | letter widths missing | Letter widths missing | The document doesn't say how wide this font's letters are, so the rest of the line couldn't be kept in place. |
| `NO_TOUNICODE` | run | Type0 or symbolic TrueType font without a usable ToUnicode map | letters not identified | Letters not identified | The document doesn't say which letters this font draws, so OffPDF can't read or retype them. |
| `AMBIGUOUS_UNICODE` | run | A glyph's text is missing, U+FFFD or private use, or ToUnicode and the glyph name disagree | letters can't be confirmed | Letters can't be confirmed | Some letters in this line can't be read with certainty, so OffPDF can't show the current text reliably. |
| `RIGHT_TO_LEFT` | run | Hebrew, Arabic, Syriac, Thaana or NKo text | right-to-left script | Right-to-left script | The document has already ordered and shaped these letters, and changing them would undo that work. |
| `COMPLEX_SCRIPT` | run | Indic scripts, Thai, Lao, Tibetan, Myanmar, Khmer, Mongolian, Hangul Jamo or any script outside the reading list | shaped script | Shaped script | This script joins or reorders letters, and the document has already done that shaping. OffPDF can't redo it yet. |
| `DUPLICATE_TEXT` | run | Another line with the same text overlaps at least half of this one's ink box | drawn twice | Drawn twice | This text is drawn twice (for example as a shadow or to look bolder), so changing one copy would leave the other behind. |
| `PER_GLYPH_TEXT` | run | At least 12 editable lines on the page and 80 % or more are one character long | letters placed one by one | Letters placed one by one | This page places every letter on its own, so a line can't be changed as one piece. |
| `NO_WRITABLE_GLYPHS` | run | The font has no character OffPDF can prove it can draw | no letters to type with | No letters to type with | The font this line uses has no letters OffPDF can confirm, so nothing can be typed with it. |
| `MALFORMED_CONTENT` | page | The page content cannot be read completely (bad syntax, an unresolvable reference, `q` nesting over 64), or two `/Contents` parts meet inside a token or comment | page can't be read | Damaged page content | OffPDF couldn't read this page's content completely, so none of its text can be changed. |
| `UNSUPPORTED_FILTER` | page | Content compressed with anything other than Flate, ASCIIHex or ASCII85, or a content stream with extra dictionary keys | unusual compression | Unusual compression | This page's content is stored or compressed in a way OffPDF can't check. |
| `PAGE_TOO_COMPLEX` | page | A per-page budget ran out (operators, glyphs, decoded bytes, page-model memory including fonts) | too much content | Too complex to check | This page has too much content to check safely. |
| `GEOMETRY` | page | `/UserUnit` other than 1, unreadable or degenerate page boxes, or a `/Rotate` that is not a multiple of 90 | custom page unit | Custom page unit | This page uses a custom unit size or unreadable page boxes, which OffPDF doesn't edit yet. |
<!-- ui-copy:end -->

Image occurrences are classified too (the #33 classifier), but v0.4 does not
edit images and does not show these codes. In priority order: `INLINE_IMAGE`,
`NESTED_FORM`, `CLIPPED`, `PATTERN`, `MASKED_IMAGE` (drawn with a mask or soft
mask), `SHARED_XOBJECT` (one image used in more than one place),
`TRANSFORMED_IMAGE` (rotated, skewed or mirrored image), `GEOMETRY`.

The codes are append-only after v0.4. The single source of truth is
[`src/lib/editor/text-reasons.json`](../src/lib/editor/text-reasons.json); see
[`EDIT_MODEL.md`](../src/lib/editor/EDIT_MODEL.md#adding-a-reason-code) for how a code is added.

## 7. Messages and Save-time errors

### While you edit

When a change cannot be committed, the editor keeps your draft and shows one of
these messages under the line. `GLYPH_MISSING` lists the characters (a space
shows as "space"). `FACE_UNAVAILABLE`, `STYLE_UNAVAILABLE` and
`TEXT_EDIT_REFUSED` show the sentence for the face, the control or the reason
involved; the table shows their default. Warnings (the last three rows) do not
block the change.

<!-- ui-copy:start -->
| Code | Message |
| --- | --- |
| `GLYPH_MISSING` | This document's font can't draw: {chars} |
| `SPACE_NOT_WRITABLE` | A space can only go between two letters here, one at a time. |
| `INVALID_TEXT` | Line breaks and tabs can't be added. Each box is a single line. |
| `TEXT_TOO_LONG` | A line can have at most 1,000 characters. |
| `TEXT_OUTSIDE_VISIBLE_AREA` | The new text would run past the edge of the visible page. |
| `FACE_UNAVAILABLE` | This page has no regular version of this font. |
| `STYLE_UNAVAILABLE` | This line's size is set in a way OffPDF can't change. |
| `EDIT_CONFLICT` | This line is already part of another change on this page. |
| `TEXT_EDIT_REFUSED` | OffPDF can't change this text safely. |
| `STALE` | “{name}” changed on disk after you started editing it. The text changes made before that can't be applied. |
| `PEN_DRIFT` | That change would move other text on this page, so it can't be saved. Undo it or try a shorter change. |
| `STATE_CHANGED` | That change would alter how other content on this page is drawn, so it can't be saved. |
| `EDIT_VERIFY_FAILED` | OffPDF couldn't confirm this change reads back exactly as typed with nothing else altered, so it can't be saved. |
| `NEXT_TEXT_OVERLAP` | The new text runs into “{neighbour}”. |
| `EDIT_NOT_VISIBLE` | This change doesn't change how the page looks. Something may be drawn over this line. |
| `PREVIEW_UNAVAILABLE` | Preview unavailable for this page. Your changes are checked again when you save. |
<!-- ui-copy:end -->

### Errors (open and Save)

Errors show a title, a message and a suggestion. At Save, every suggestion ends
with the sentence that the original file was not changed (it is added in one
place, `reasons::save_failure`, and asserted by a Rust test). A file that cannot
be opened for Edit text shows its error in a banner instead; the rest of Edit
PDF keeps working. An empty suggestion is shown as "—" here.

When a file is opened:

<!-- ui-copy:start -->
| Code | Title | Message | Suggestion |
| --- | --- | --- | --- |
| `ENCRYPTED` | This PDF is password-protected | OffPDF can't change text in a protected PDF. | Remove the password with Unlock PDF, then edit the unlocked copy. |
| `SIGNED` | This PDF is digitally signed | Changing its text would break the signature. | Ask the sender for an unsigned copy if it needs changes. |
| `UNSUPPORTED_XFA` | This PDF is a dynamic form | Its pages are generated by the PDF reader, so changes to page text might not show. | Fill it in a reader that supports dynamic forms. |
| `PDF_NEEDS_REPAIR` | This PDF needs repair first | Its internal structure has errors, so OffPDF won't change its text. | Run it through Repair PDF, then edit the repaired copy. |
| `FILE_TOO_LARGE` | This PDF is too large to edit text in | Files over 400 MB are not read for text editing. | Split it with Split PDF and edit the part you need. |
| `FILE_TOO_COMPLEX` | This PDF is too complex to check | Some of its internal data is too large or too deeply nested to check safely. | — |
| `MALFORMED_CONTENT` | Part of this PDF can't be read | OffPDF couldn't read this PDF's structure completely. | Run it through Repair PDF, then try again. |
| `VERIFIER_MISSING` | A checking component is missing | OffPDF needs its bundled Poppler tools to verify text changes. | Reinstall OffPDF, then try again. |
| `INVALID_PDF` | The selected file is not a valid PDF | OffPDF could not open this file as a PDF document. | Make sure the file is a real PDF and is not corrupted. |
| `ENGINE_MISSING` | PDF engine not found | The bundled qpdf engine could not be located. | Reinstall OffPDF, then try again. |
<!-- ui-copy:end -->

`PDF_NEEDS_REPAIR` can also arrive later: the full `qpdf --check` runs in the
background after the file opens, and its result is needed before the first
preview and before Save. When the bundled qpdf is older than version 11,
`ENGINE_MISSING` says that the engine is too old instead. If qpdf cannot open
its copy during the check (for example after the app's temporary files were
cleared), that is an engine failure (`ENGINE_FAILED`), never a repair verdict,
and the check runs again at the next preview or Save.

When a change cannot be saved (the same checks run at preview and at Save;
`{n}` is the page number in the saved file):

<!-- ui-copy:start -->
| Code | Title | Message | Suggestion |
| --- | --- | --- | --- |
| `GLYPH_MISSING` | The font can't draw some letters | On page {n}, the document's font can't draw: {chars} | Use other characters, or add a new text box with Add text. |
| `SPACE_NOT_WRITABLE` | A space can't go there | On page {n}, spaces in the changed line are gaps between letters, so a space can only go between two letters, one at a time. | Remove the extra space. |
| `INVALID_TEXT` | These characters can't be added | Line breaks, tabs and control characters can't be added. Each box is a single line. | Remove them and try again. |
| `TEXT_TOO_LONG` | Text is too long | A changed line can have at most 1,000 characters. | Shorten the line. |
| `TEXT_OUTSIDE_VISIBLE_AREA` | Text runs off the page | On page {n}, a changed line would run past the edge of the visible page. | Shorten the line. |
| `FACE_UNAVAILABLE` | That style isn't available | On page {n}: {sentence} | Keep the current style. |
| `STYLE_UNAVAILABLE` | That change isn't available | On page {n}: {sentence} | Keep the current setting. |
| `EDIT_CONFLICT` | Two changes overlap | Two text changes on page {n} affect the same line. | Undo one of them and try again. |
| `TEXT_EDIT_REFUSED` | This text can't be changed safely | On page {n}: {body} | Restore the original text for that line. |
| `STALE` | The PDF changed on disk | “{name}” was changed after you started editing it, so your text changes no longer match it. | Remove these text changes and make them again. The original file was not changed. |
| `PEN_DRIFT` | The change would move other text | Saving would shift text you didn't change on page {n}, so nothing was saved. | Try a shorter change, or undo it. The original file was not changed. |
| `STATE_CHANGED` | The change would restyle other content | Saving would change how other content on page {n} is drawn, so nothing was saved. | Undo the last change on that page and try again. The original file was not changed. |
| `EDIT_VERIFY_FAILED` | The change could not be verified | OffPDF checks every change before saving. On page {n}, a changed line couldn't be confirmed to read back exactly as typed with nothing else altered, so nothing was saved. | Undo that change and try again. The original file was not changed. |
<!-- ui-copy:end -->

Only at Save:

<!-- ui-copy:start -->
| Code | Title | Message | Suggestion |
| --- | --- | --- | --- |
| `SOURCE_EDIT_GATE_FAILED` | The edited PDF did not pass the text-change check | The saved file did not contain the text changes exactly as they were checked, so it was not published. | Try saving again. The original file was not changed. |
| `TEXT_EDIT_ON_REDACTED_PAGE` | Redaction and text change on the same page | Page {n} has both a redaction and a text change. Redaction turns the page into an image, so the text change would be lost. | Remove the redaction or the text change on that page. |
| `TEXT_EDIT_DUPLICATE_PAGE` | This page appears twice | Page {p} of “{name}” is in the list more than once and has a text change. | Remove the extra copy of the page, then save again. |
| `TOO_MANY_TEXT_EDITS` | Too many text changes | This save has more than 500 changed lines. | Save in smaller batches. |
| `INVALID_PAGES` | Invalid page selection | This page is not in the PDF. | Open the page again from the page list. |
| `BAD_EDIT` | Could not save | A text change has a setting OffPDF can't use. | Undo the last change and try again. |
<!-- ui-copy:end -->

`TOO_MANY_TEXT_EDITS` is also returned for more than 200 changed lines on one
page. Every job command (Save and the other Edit PDF and tool jobs) accepts only
job ids of 1–128 ASCII letters, digits, `-` and `_`; anything else is
`INVALID_JOB` ("Invalid job" / "OffPDF received a job id it does not accept.",
no suggestion), so a job's work folder stays inside the app's temp area. Files
larger than 100 MB open with a note that checking and saving takes longer. The
technical details of every error (check id, page, byte offset, drift in points)
are collapsed under the message.

## 8. How a change is written

A change is a byte splice inside the page's own content stream. Nothing else
in the file is rewritten by OffPDF; qpdf writes the result.

```text
user's PDF ──one capped read──► snapshot bytes (hashed; qpdf only ever reads copies of these)
                                   │ lopdf parses the same bytes, read-only
                                   ▼
  /Contents parts ──bounded decode──► joined buffer ──byte-offset lexer──► show ops ──► lines (runs)
                                                                                 │
                     plan_page: minimal diff + pen compensation  ◄───────────────┘
                                   │
                                   ▼
         splices: byte ranges of the line's show ops → replacement bytes
                                   │ applied back to front, per part
                                   ▼
         expected parts (the exact decoded bytes every edited part must have)
                                   │ self-check: the same walker re-reads the expected parts
                                   ▼        before any file is written
         update.json  {"obj:N G R": {"stream": {"dict": {}, "data": "<base64>"}}}
                                   │
                                   ▼
  qpdf source.pdf edited.pdf --decode-level=none --compress-streams=n --update-from-json=update.json
                                   │
                                   ▼
                  Phase A gate on edited.pdf (section 9)
```

**Minimal diff.** The unchanged prefix and suffix of the line keep their glyph
codes, fonts and the kerns between them, byte for byte. Only the middle is
re-encoded, with the font's own codes. A kern at a boundary that was tuned for a
glyph pair that no longer exists is dropped; a kern that reads as a word space
is kept. Worked example (a Word-style line, test PLAN-04):

```text
before: BT /F1 11.04 Tf 1 0 0 1 72 700 Tm [(Inv)12(oice 2026)]TJ ET
after:  BT /F1 11.04 Tf 1 0 0 1 72 700 Tm [<496E76> 12 <6F6963652032303237>] TJ ET
```

"Invoice 2026" became "Invoice 2027"; the kern 12 between "v" and "o" is kept;
digits have equal widths, so no compensation number is written.

**Compensation.** With `A` the line's original advance and `A′` the new one (in
text space, including `Tc`, `Tw` and kerns), the pen is restored by one number at
the end of the last `TJ`:

```text
n_c = (A′ − A) × 1000 / Tf′        (omitted when |n_c| < 0.0005)
```

Numbers are written with at most 4 decimals and parsed back; all later
arithmetic uses the parsed value. If the kept suffix holds a column gap of at
least 1 em, that gap absorbs the change instead, and the text after it keeps its
original position.

**Style.** A size change writes a scaled `Tf`; a colour change writes `rg`; a
letter-spacing change writes `Tc`. After the line, the original operators are
restored verbatim (the original `Tf`, colour-space and colour operators, `Tc`
source bytes), so later content is drawn exactly as before.

**Absorbed pieces.** When a line was joined from several show operators, the
first one draws the whole new line and each other piece becomes `[<> n] TJ`,
which draws nothing and keeps that piece's original pen travel
(`n = −advance × 1000 / Tf`).

**Replacement grammar.** A replacement may contain only `Tf Tc Tw T* TJ`, the
verbatim restore operators (`g rg k cs sc scn`) and the new `rg`. It never
contains `q Q BT ET cm Tm Td TD Tz Ts Tr gs`, paths, images or marked content.
This is checked before any file is written and again on the re-read.

**Why qpdf writes.** lopdf 0.34's incremental writer would add a second header
line and re-print numbers through `f32`. qpdf's `--update-from-json` replaces
only the data of the edited streams, keeps every untouched filtered stream raw
(`--decode-level=none`) and drops the superseded data, so the old text cannot be
recovered from the new file (test E2E-18).

## 9. Verification

Every change passes the same checks at preview and at Save. A failure publishes
nothing.

**Plan self-check (before any IO).** The planner re-walks the expected bytes
with the same walker and runs the re-walk checks below. A failure is an internal
`EDIT_VERIFY_FAILED`; qpdf is never started.

**Phase A** runs on each edited copy (and on the preview file). The copy is
opened with the same resource bounds as a source, but without the policy
refusals (`read_verification_snapshot`).

| Check | Proves | Catches |
| --- | --- | --- |
| A0 qpdf | `qpdf --check` exits 0, or 3 with only warnings the source already had | a mis-targeted update (qpdf writes it with exit 0 and only `--check` notices) |
| A1 engines agree | lopdf and qpdf list the same pages and `/Contents` ids; same page count | lopdf misreading qpdf's output; dropped or duplicated pages |
| A2 whole graph | Every object reachable from `/Root` and `/Info` equals qpdf's input, except the edited content streams, which decode to exactly the expected parts. Object ids are ignored; filtered streams are compared raw. | collateral damage anywhere: other pages, fonts and `/Widths`, resources, `/Annots`, boxes and `/Rotate`, page order, catalog (layers, forms, names, open action, outlines), added content streams or Form XObjects, raster replacements, shared-stream changes |
| A3 edited parts | Same part count; each part decodes to its expected bytes; the original bytes are not attached anywhere on the page | off-by-one splices, the wrong page, re-encoded content, the old stream re-attached |
| A4 re-walk | Every unedited glyph keeps its codes, font, text and state, and stays within 0.01 pt; every paint keeps its state; the edited line has exactly the planned glyphs, origin, size, spacing, colour and pen; the state after the line equals the original | wrong compensation, state leaks (`Tc`, `Tz`, colour, line width, …), missing glyphs, wrong size or font |
| A5 independent | Poppler (`pdftotext -bbox`) finds the same words outside the edited band; inside it, the words of the line's other runs read the same at the same place (or joined to the new text at their outer edge) and are set aside, then the new text is found and the old words fewer times. A word holding glyphs of a run no edit changes is such a neighbour wherever it lies (a footnote marker or a "." right after the line), and a word Poppler joins across the edit ("Hello:") is split, its other run's part pinned at the outer edge (or, when a new letter covers it, found inside the new word). `pdftoppm` renders of the page before and after differ in at most 8 pixels outside the edited glyph boxes and in at most 2 pixels of other runs' glyphs outside the edited glyphs' own boxes (grown by 1 pixel), and no neighbour under the new glyphs loses more than 2 pixels of ink | width-model errors that OffPDF's own walker would share, old text still extractable, rendering-level leaks |

**Phase B** runs on the final file after every other Edit PDF pass (assembly,
stamps, links, forms, markup, redaction on other pages), just before the existing
output validation (#34). Any failure is `SOURCE_EDIT_GATE_FAILED`.

| Check | Proves |
| --- | --- |
| B0 read | The final file opens within bounds and lopdf and qpdf agree on its pages |
| B1 present | Each edited page holds its expected content exactly once, either as a contiguous run of content parts, or inside the one Form XObject qpdf's `--overlay` wraps a page in (identity transform, `/BBox` containing the visible page, and qpdf's join of the expected parts token-neutral as below: PB-09) |
| B2 absent | The original bytes of the edited parts are nowhere on the page, its wrapper or its Form XObjects |
| B3 re-walk | The page's text records equal the proven records one to one (codes, fonts, text, origins within 0.01 pt, full state); later passes may only add paints |

**Composition with #34.** After Phase B, the #34 snapshot digest of each edited page
must equal the digest computed by the planner, independently of the files on disk.
#34's own check then runs unchanged. For a page whose parts do not end with a line
break it also accepts qpdf's joined form (`alt_content_digest`), only in saves qpdf
overlays (any text box, image, shape, drawing or markup) and only if the join is
token-neutral: every newline qpdf adds falls between tokens, outside comments,
strings, inline images (and the 64 bytes after one) and `BX`…`EX` sections, and
where two regular characters meet, the token before is a number they cannot continue
or an operator the next 1–3 characters do not extend into another (`ET`|`BT` and
`0`|`cm` pass; `s`|`h`, `12`|`3`, `/F1`|`2` do not); the walker uses the same rule
(`content/joins.rs`). Otherwise #34 refuses the save (`INVALID_OUTPUT`), with or
without text changes, so a join that would show text the original hid (a line
commented out across a part boundary) is never published. Before v0.4 any such page
next to a stamp failed validation (VO-01, E2E-10c, `tests_e2e/join.rs`).

**Why a painted-over fake fails.** Tests build each fake with the test kit or real
qpdf passes and show that each listed check fails on its own:

| Fake | Fails at |
| --- | --- |
| Original part kept, a white box and new text appended (GATE-18) | A2, A3, A4, A5; Phase B B1, B2, B3 |
| Real `qpdf --overlay` of a white box and text page (GATE-19) | A2, A3, A5; B1, B2 |
| Page replaced by a rendered image (GATE-20) | A2, A3, A4 |
| FreeText annotation over the old words (GATE-22) | A2 |
| Old stream re-attached as an unused Form XObject (GATE-24) | A2, A3; B2 |
| Wrapper Form that still holds the old parts (PB-05) | B1, B2, B3 |
| Wrapper whose `/BBox` cuts the page (PB-06) | B1 |
| Fake injected into the edited copy during a real Save (GATE-25) | Phase A; nothing is published |

#34 alone passes the first fake, because the original content is still present
(GATE-21). That is why Edit text has its own gate.

**Independent engines.** Poppler is used in the app, not only in tests, so a
mistake in OffPDF's width or encoding model cannot verify itself. If the Poppler
tools are missing, Edit text refuses with `VERIFIER_MISSING`; it never skips the
check. Frames are pinned for `/Rotate` 0/90/180/270 and offset CropBoxes (IND-06).

## 10. Limits and budgets

All limits are constants in `src-tauri/src/pdf_engine/text_edit/limits.rs`.
Exceeding a page budget refuses that page (`PAGE_TOO_COMPLEX`) and leaves other
pages editable. Exceeding a file budget refuses the file. Inside the gate, a budget
failure is `EDIT_VERIFY_FAILED` ("too large to verify"), never a skipped check.

| Area | Limit |
| --- | --- |
| File | 400 MiB (`FILE_TOO_LARGE`); 2,000,000 objects; xref chain of 64 sections; object nesting 100; object streams 32 MiB each and 256 MiB in total decoded; xref streams 64 MiB decoded (`FILE_TOO_COMPLEX`) |
| Any stream | 32 MiB decoded, capped while inflating |
| Page content | 48 MiB decoded; 256 parts; 250,000 operators; 400,000 glyphs; 20,000 lines; 256 fonts; `q` nesting 64; Form nesting 8; 96 MiB decode budget for content, forms, fonts and CMaps of one page |
| Page model | 160 MiB per page for the model, the font models it keeps and its #33 Classify pass together (the lexed operators are dropped once the page is walked). Unused fonts of the page's resources are kept only within 32 MiB and a quarter of the room left, else left out. At most about 131,000 one-glyph shows under one state; 3,000 lines with a `Tm` per glyph hold 89–112 MiB |
| Fonts | program 16 MiB decoded; ToUnicode 2 MiB and 131,072 mappings; `/W` 65,536 entries |
| Edits | 1,000 characters per line; 200 changed lines per page; 500 per Save |
| Tolerances | glyph drift 0.01 pt; join baseline 0.01 pt; join gap ±0.3 em; Poppler word boxes 0.05 pt; pixel channel difference 24; at most 8 differing pixels outside the edited glyph boxes; at most 2 in other runs' glyph boxes outside the edited glyphs' own boxes (grown by 1 pixel), and 2 of a neighbour's ink lost under the new glyphs |
| Rendering for A5 | at most 96 DPI and 25 million pixels per page; a page that would render below 25 DPI fails closed |
| Subprocesses | 120 s timeout each; `pdftotext` output 16 MiB; qpdf JSON 64 MiB |
| Preview | one-page file of at most 48 MiB, else "Preview unavailable" (Save still checks) |
| Cache | 2 open sources, each kept only up to 256 MiB (calls on a larger file that overlap share one read); per source 32 page models and 256 MiB of them, fonts included; a model over 64 MiB leaves the cache when previewed and is built again on the next visit |
| Save | Page models are kept from planning to Phase A within 2 × the file size (at least 16 MiB); once one does not fit, none is kept and Phase A builds each again, one at a time. The source's and the verification copies' raw bytes are dropped once read |

Measured by the test suite on 2026-10-03 (debug build): a page of 100,000
one-glyph shows holds an 84 MiB model and its Classify pass 22 MiB more; planning
an edit on it adds 103 MiB, and its preview peaks at 209 MiB for the whole
process; saving three pages of 30,000 shows peaks at 66 MiB (one page: 54 MiB).

## 11. Compatibility matrix

Expected behaviour per producer. Each row is backed by a synthetic,
producer-shaped fixture built in memory by the test kit (no third-party PDFs are
committed). Real files are checked with the manual protocol in
[section 17](#17-manual-test-protocol).

| Producer | Typical structure | v0.4 | Evidence |
| --- | --- | --- | --- |
| Microsoft Word, Latin | TrueType subset, WinAnsi, `/Widths`, ToUnicode, kerned `TJ` per format run, page-sized clip, tagged (`/MCID`), bookmarks | Editable; letters limited to the subset | FX-WORD: E2E-03, PLAN-04, IND-01, RUN-01 |
| Word, Turkish / Polish / Czech | the same plus a Type0 companion font for non-WinAnsi letters | Editable as one line (sibling join); ğ ş ı İ when present in either subset | FX-WORD-TR: E2E-04, RUN-05, IND-01 |
| Tagged and bookmarked files (Word's default) | StructElem `/Pg`, annotation `/P`, outline and link destinations, `/OpenAction` | Editable (these references do not count as sharing) | CON-07, CON-08, CLS-H27, E2E-22 |
| Word-style hybrid xref | object stream listed in the classic table and the `/XRefStm` | Editable when lopdf and qpdf agree; otherwise `PDF_NEEDS_REPAIR` | SNAP-16, E2E-22; SNAP-11, SNAP-12 |
| LibreOffice | symbolic TrueType subset, hex `TJ` with kerns | Editable per joined line | FX-LIBRE: E2E-05, IND-01 |
| LibreOffice with OpenType-CFF fonts | `/Type1C` or `/OpenType` | Editable | FX-LIBRE-CFF: E2E-05, IND-01 |
| Google Docs, Chrome and Edge print | Type0 Identity-H subset, flipped `cm`/`Tm`, one operator per text node, synthetic bold (render mode 2) and italic (shear) | Editable; colour unavailable for synthetic bold | FX-SKIA: E2E-05, IND-01, GEO tests |
| Firefox, Safari / Quartz | symbolic subsets with ToUnicode, sometimes one glyph per operator | Editable; per-glyph operators on one line join (a page that stays one glyph per line is `PER_GLYPH_TEXT`) | FX-QUARTZ, FX-PERGLYPH: E2E-05, IND-01, RUN-12 |
| pdfTeX / LaTeX | Type1 subsets with `/Differences`, often no ToUnicode or space glyph, word gaps as kerns | Editable (glyph names, kern spaces); math fonts usually `AMBIGUOUS_UNICODE` | FX-PDFTEX: E2E-05, RUN-07, IND-01 |
| XeLaTeX / LuaLaTeX | Type0 Identity-H CIDFontType0C | Editable | FX-XETEX: E2E-05, IND-01 |
| InDesign / Illustrator | CFF fonts, tracking, layers, some text in Forms | Editable on visible layers; text in Forms listed as refused lines (`NESTED_FORM`); hidden layers `OPTIONAL_CONTENT` | FX-INDD: E2E-05, IND-01; corpus `text-nested-form` |
| Report generators (FPDF, TCPDF, ReportLab, wkhtmltopdf) | Standard 14 not embedded, or TrueType subsets | Editable, with the right widths per face | FX-STD14: E2E-01, E2E-02, E2E-05, CLS-H18 |
| Office exports without embedded fonts | simple TrueType with `/Widths`, no program | Editable, marked "not included in the PDF" | FX-NONEMB: E2E-05 |
| Scanner + OCR | image plus invisible text | `INVISIBLE_TEXT` | FX-OCR; CLS-H13 |
| CAD, plotting, design tools | Type3, outlines, patterns, deep Forms, `/UserUnit` | `TYPE3`, `PATTERN`, `NESTED_FORM`, `GEOMETRY` or no text | corpus `text-type3`, `text-nested-form`; GEO-12…15 |
| Imposition, letterhead templates | shared content streams | `SHARED_CONTENT` | FX-SHARED: CLS-H17, GATE-14 |
| Invoices and tables | each cell its own show operator on a shared baseline | Editable; a longer cell that runs into the next saves with `NEXT_TEXT_OVERLAP` | IND-11, `tests_e2e/save.rs` (B2) |
| Content split into parts without line breaks | `/Contents` arrays whose parts meet without whitespace | Editable when the parts meet between tokens (`ET`\|`BT`, `0`\|`cm`, a comment's end of line); a boundary inside a token, comment or `BX`…`EX` section, or within 64 bytes after an inline image → `MALFORMED_CONTENT` | E2E-10c, VO-01, `tests_walk/joins.rs`, `tests_e2e/join.rs`, PB-09 |
| Pages sharing inherited `/Resources` | a bold face only in the shared dictionary | Editable; preview and Save agree | PREV-06 |
| Legacy filters on other pages | RunLength or LZW content left untouched | Editable pages unaffected; those pages keep their raw bytes; an edited page with such content is `UNSUPPORTED_FILTER` | APP-06, GATE-29 |
| Signed, encrypted, dynamic XFA | — | `SIGNED`, `ENCRYPTED`, `UNSUPPORTED_XFA` at open; an unedited signed file may be appended | SNAP tests, E2E-13, E2E-20 |
| qpdf 11.9 (CI) and 12.x (bundled, dev) | `--overlay` wraps pages in a Form; content parts are joined with qpdf's rule | Both forms accepted by Phase B; the join rule and JSON shapes are pinned on whichever qpdf is installed | APP-07, PB-02, CON-10, ENG-07 |

## 12. Performance and engine matrix

Measured on an Apple M5 (10 cores, 24 GiB), macOS 26.4.1, rustc 1.96.0, qpdf
12.3.2, Poppler 26.04.0, release build, **2026-10-03**, on the final v0.4 code.
Other builds shared the machine (load average 30–38 on 10 cores), so times are
pessimistic; peak RSS is not affected.

**Engine matrix (the four #33 axes).**

| Path | Binary size | Memory (peak RSS) | Build and signing cost | Native crash isolation |
| --- | --- | --- | --- | --- |
| Shipped: lopdf 0.34 (read only) + own lexer and walker + qpdf writer + Poppler verifier | macOS arm64 executable 12,866,048 → 13,991,552 bytes (+1,125,504 bytes, +8.7 %); bundled tools unchanged, so this is the whole bundle delta. Windows: not measured | 51 MiB (10 pages), 86 MiB (100), 114 MiB (1,000 pages); 639 MiB for a 305 MiB image-heavy file (2.10 × the file) | No new native binary: `qpdf`, `pdftotext` and `pdftoppm` are already bundled and signed (`scripts/sign-macos-bundled-tools.sh`, `scripts/prepare-poppler-windows.ps1`). No new crate; `flate2` moved from dev-dependency to dependency (MIT/Apache-2.0, already compiled in through lopdf). | qpdf and Poppler run out of process with a 120 s timeout. lopdf parses in process under `panic = "abort"`, behind a raw preflight (xref chain, nesting, `/Length` chains), object-stream guards, per-page budgets and fuzzing. Residual risk: lopdf's parser on inputs within those bounds (the same exposure every existing lopdf feature has). |
| PDFium sidecar | not prototyped | not prototyped | not prototyped (would add a per-platform native binary to build, sign and notarise) | not prototyped |

BENCH-SIZE: `src-tauri/target/release/offpdf` from `cargo build --release -j 6`
(shipped profile: `panic = "abort"`, LTO, one codegen unit, `opt-level = "s"`,
stripped). Before: 2026-09-30, the tree before the first v0.4 change; after:
2026-10-03, the final code. The delta includes the embedded new editor UI.

**BENCH-01** (generated documents, 40 Helvetica lines per page; inspect over 20
sampled pages, cold = first visit, warm = cached; preview of one edit on 10
pages; Save with N changed lines spread over the pages; times in ms):

| Pages | File KiB | Open | Inspect cold p50 / p95 | Inspect warm p50 / p95 | Preview p50 / p95 | Save 1 | Save 10 | Save 100 | Peak RSS MiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 10 | 29.8 | 25 | 1 / 3 | 0 / 0 | 360 / 399 | 416 | 2,341 | 2,329 | 51 |
| 100 | 298.7 | 32 | 1 / 2 | 1 / 2 | 369 / 388 | 440 | 2,321 | 21,979 | 86 |
| 1,000 | 3,026.1 | 86 | 2 / 4 | 0 / 1 | 346 / 513 | 1,069 | 2,630 | 22,969 | 114 |

**BENCH-02** (a 305 MiB file: 20 text pages and 31 image pages, each with a
5 MiB Flate image and a JPEG):

| File MiB | Open ms | First inspect ms (after open) | Background `qpdf --check` done ms (from open) | Preview ms | Save 1 edit ms | Peak RSS MiB | RSS / file |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 305 | 1,555 | 1,345 | 4,256 | 1,713 | 11,850 | 639 | 2.10 |

Budgets: first inspect ≤ 3 s after open (1.3 s), Save ≤ 90 s (11.9 s), peak RSS
≤ 3 × file (2.10 ×). All met.

What the numbers mean for users:

- Opening is fast; the full `qpdf --check` runs in the background and does not
  block the first inspect.
- Save time grows with the number of edited **pages**, not lines (Phase A runs
  Poppler twice per edited page, about 0.2 s each here): 100 pages took about 22 s.
- Files above 100 MB work but check and save more slowly, which the app says when
  the file opens; files above 256 MiB are re-read by later visits (section 15).

## 13. Engine decision

**lopdf 0.34 (read only) + OffPDF's own lexer and walker + qpdf as the writer +
Poppler as the independent verifier. No PDFium sidecar.**

- **Byte-exact splices need a byte-offset view of the content.** lopdf's
  `Content::decode`/`encode` lose the original bytes, and `Content::decode`
  returns a silent prefix for a stream with garbage in the middle (test LEX-20),
  so OffPDF reads content with its own bounded lexer.
  These lopdf calls are forbidden in the module and a test enforces it.
- **PDFium's editing API regenerates page content** (`FPDFPage_GenerateContent`),
  the opposite of a byte-exact splice, so it could not keep unchanged glyphs and
  kerns byte for byte.
- **A sidecar adds a per-platform native binary** to build, sign and notarise.
  The shipped path adds none: qpdf and Poppler are already bundled and signed.
- **Crash isolation is obtained differently.** Everything that writes or
  independently verifies runs out of process (qpdf, Poppler). lopdf only reads,
  from bounded, preflighted bytes, with per-page budgets and fuzzed parsers.
  Moving the lopdf read into a helper process is a v0.5 option.
- **lopdf and qpdf must agree.** Because lopdf 0.34 can read some files
  differently from qpdf (pages without `/Type`, hybrid xref sections), every
  file is checked for page-map agreement at open and on every file the gate reads;
  disagreement is `PDF_NEEDS_REPAIR`.

The #33 criterion "if PDFium is tested, it runs out of process" is not
triggered: PDFium was not prototyped (PR #98 honesty convention).

## 14. UX wording rules

- Allowed verbs: "change", **Edit text**, **Add text**. Edit text changes words
  already in the PDF; Add text places a new text box.
- Never in the UI: "cover", "white-out", "flatten", "overlay", "edit like Word",
  "we'll". `sourceTextCopy.test.ts` scans every exported string, and
  `docsReasons.test.ts` scans the quoted copy blocks of this page.
- Every refusal names its reason (sections 6 and 7); nothing is greyed out
  without a sentence.
- Every Save failure ends with the sentence that the original file was not
  changed.
- All copy lives in `src/lib/editor/sourceTextCopy.ts`; Rust builds the
  `AppError` copy in `text_edit/reasons.rs` with the same text.

Examples as shipped:

<!-- ui-copy:start -->
| Key | Sentence |
| --- | --- |
| `UI.tool.editText.title` | Edit text (E): change words already in this PDF, in its own font |
| `UI.tool.addText.title` | Add text: place a new text box on the page |
| `UI.banner.mode` | Select an outlined line to change its words. Changes use the document's own font, and the rest of the page stays exactly where it is. Dotted lines can't be changed safely — select one to see why. |
| `UI.banner.addText` | Adds a new text box. To change words already on the page, use Edit text (E). |
| `UI.popover.title` | This text can't be changed safely |
| `UI.popover.footer` | You can add a new text box on top with Add text. It won't replace this text. |
| `UI.outputAlert` | Text changes use the document's own font. Before saving, OffPDF checks that nothing else on those pages moved by more than 0.01 pt. The original file is never changed. |
| `UI.substituted` | This font isn't included in the PDF. Your PDF reader draws it with a similar font. |
| `UI.removal` | This line will be removed. Text after it stays where it is. |
| `UI.blocked` | Fix or cancel this change first. Press Esc to cancel. |
| `UI.face.noBold` | This page has no bold version of this font. |
| `UI.face.noItalic` | This page has no italic version of this font. |
| `UI.face.cannotDrawBold` | The bold version of this font can't draw: {chars} |
| `UI.style.size` | This line's size is set in a way OffPDF can't change. |
| `UI.style.face` | This line's font is set in a way OffPDF can't change, so bold and italic aren't available. |
| `UI.style.colour` | This text is drawn with an outline, so its colour can't be changed here. |
| `UI.status.previewUnavailable` | Preview unavailable for this page. Your changes are checked again when you save. |
| `UI.notVisible` | This change doesn't change how the page looks. Something may be drawn over this line. |
<!-- ui-copy:end -->

## 15. Known limitations and v0.5 candidates

Limitations of v0.4, in plain terms:

- **A damaged JPEG anywhere in the file blocks preview and Save** with
  `PDF_NEEDS_REPAIR`. `qpdf --check` decodes every JPEG, and "JPEG data is
  corrupt" is not on the allow-list of harmless warnings (only linearization,
  hint-table and `/Size` warnings are). Scanner output with one damaged image is
  affected. Run the file through Repair PDF first.
- **Reading a page and previewing a change cannot be cancelled** (Save can). The
  part-boundary scan, also in Save's checks, cannot be stopped and inflates each
  Flate inline image again after the page's lex (debug: 4.9 s per 96 MiB of plain
  operators; 0.76 s for 64 images of 16 MiB, so ~70 s for a crafted 96 MiB page).
- **Files over 256 MiB are not kept between calls.** Calls that overlap share one
  read; a later page visit or preview reads the file again (about 1.5 s at
  305 MiB on the bench machine).
- **Very dense pages are refused** (`PAGE_TOO_COMPLEX`) when the model, its fonts
  and its #33 Classify pass would hold more than 160 MiB (about 131,000 one-glyph
  shows under one state). A font used by several pages counts in full on each;
  unused fonts past their share are left out, so fewer sibling faces may be
  offered for bold and italic. An ExtGState with more than 64 keys, or key names
  longer than 127 bytes, also refuses the page.
- **The part-boundary check is conservative**: a `/Contents` part boundary inside an
  inline image, a hex string or a `BX`…`EX` section, or within 64 bytes after an
  inline image, refuses the page (`MALFORMED_CONTENT`), even at whitespace.
- **A longer line that runs into the next text** is checked by Poppler for that
  text's words, place and ink, but where the new letters cover it not its colour or
  outline (a neighbour turned red or blue, or drawn stroked) nor a small shift under
  one letter: OffPDF's own re-walk (A4), which compares every glyph's colour, render
  mode and position, catches those.
- **Rarely, a correct overlapping change is refused** (`EDIT_VERIFY_FAILED`): when
  the new letters run over the next text in the colour of the paper behind it
  (white letters from a dark band into black text on white), the pixel check
  cannot tell that from the neighbour being erased.
- **Only letters the embedded font contains** can be typed; subset fonts usually
  hold only the letters the document already uses. Ligature glyphs read as their
  letters but are never typed.
- **One line at a time**, one style per line: no reflow, wrapping, per-letter
  styling, new fonts, underline or stroke colour.
- **Very large pages are checked at a lower resolution.** The pixel check renders
  at most 25 million pixels: pages larger than about 52 in on a side are rendered
  below 96 DPI, so it sees less detail (the 0.01 pt re-walk still applies), and a
  page that would render below 25 DPI (around 200 in on a side) fails closed with
  `EDIT_VERIFY_FAILED`.
- **Type1, non-embedded and some CFF fonts** use a conservative box instead of the
  exact glyph outline for the new letters in the pixel check; a neighbour within
  about 0.2 em of them is checked for its words and ink there, not its colour.
- **Temporary copies.** Each open file has a copy in the app's temp folder
  (`textedit/`), deleted when the file is closed or evicted; a running preview keeps
  it until it ends, and copies a crash left behind are deleted at the next start.
- **Reading order** for the keyboard follows the structure tree on tagged pages; on
  untagged pages it is a heuristic (XY-cut). Lines drawn through Form XObjects come
  after the page's own lines, and are listed only when the page's #33 check
  completes.
- **A page listed twice** in the file list, or a page with a redaction in the same
  Save, cannot take text changes.
- **The lopdf parser runs in process.** It only reads, within the preflight and
  budgets above; a malformed file that passes them shares the exposure of every
  existing lopdf feature.

Out of v0.4 by decision (never faked, always refused with a reason): image
replacement, text inside Form XObjects, annotation and form-widget text,
shared content streams, Type3, vertical, right-to-left and complex scripts,
`/UserUnit` ≠ 1, reflow and multi-line editing, font embedding or substitution.

v0.5 candidates:

- copy-on-write for shared content streams through qpdf JSON page dictionaries
  (probe P2 shows qpdf can write a new stream and drop the old one);
- text inside Form XObjects (copy-on-write of the Form per page);
- Type3 fonts;
- existing-image replacement (needs its own spike: shared XObjects, masks, size
  policy);
- render only the edited bands at full resolution for very large pages;
- cancellable inspect and preview;
- the lopdf read in a helper process;
- a decision on whether DCT decode warnings from `qpdf --check` may be treated as
  harmless for files whose damaged image is not on an edited page.

## 16. Contributor notes

### Module map

Rust, `src-tauri/src/pdf_engine/text_edit/` (no file over 800 lines; children in
same-named folders):

| Layer | Files |
| --- | --- |
| Core | `limits.rs` (every budget), `reasons.rs` (codes and `AppError` copy), `snapshot.rs` (+ `snapshot/{preflight,objects,headers}.rs`: one capped read, preflight, guarded lopdf load, page-map agreement), `decode.rs` (capped Flate, ASCIIHex, ASCII85), `lexer.rs` (byte-offset tokenizer, inline-image proofs), `content.rs` (+ `content/joins.rs`: parts, joined buffer, qpdf join rule, part-boundary check, ownership), `engines.rs` (tools, subprocesses with timeout and cancel, `qpdf --check` classification and memo) |
| Fonts | `fonts/` (encodings, AGL, Standard 14 data, ToUnicode, glyph presence for TrueType, CFF, Type1; faces and typing surfaces) |
| Page model | `context.rs`, `geometry.rs` (strict boxes and orientation), `state.rs`, `structure.rs` (optional content, ActualText, structure order), `walker.rs` (+ `walker/`, incl. the per-page byte budget and font charging), `runs.rs` (+ `runs/`, incl. `forms.rs`: Form XObject lines), `order.rs` |
| Write | `encode.rs` (numbers, grammar), `rewrite.rs` (+ `rewrite/`: `plan_page`), `fit.rs` (word spacing; `fit/estimate.rs`, test-only, is the width estimate the frontend golden is checked against), `apply.rs` (qpdf update) |
| Verify | `verify.rs` (re-walk), `graph.rs` (canonical whole-graph digest), `poppler.rs` (+ `poppler/words.rs`: words and neighbours; `poppler/ink.rs`: neighbour ink; `poppler/near.rs`: the model's glyphs around the edit), `gate.rs` (+ `gate/`: Phase A, Phase B, `join.rs` token-neutral joins), `preview.rs` |
| Integration | `cache.rs`, `dto.rs`, `service.rs`, `export.rs` (+ `export/map.rs`: Save), `commands/text_edit.rs` (4 Tauri commands) |
| #33 classifier | `pdf_engine/source_content.rs`, an adapter over the page model; `service::inspect_page` builds every "can / can't be changed" from it |

Frontend: `src/lib/editor/{sourceText,sourceTextCopy}.ts` (pure helpers, copy),
`src/lib/editor/text-reasons.json` (codes), `src/lib/types.ts` (IPC types),
`src/lib/tauriCommands.ts` (IPC), `src/features/edit-pdf/{useTextSources,textSaveGuards}.ts`,
`src/components/pdf/editor/sourceText/` (mode, layer, editor, format bar, reason
popover, page text and preview hooks), `src/styles/source-text.css`.

### Forbidden APIs

New code never calls lopdf's `Content::decode`/`encode`, `string_to_bytes`,
`replace_text`, `encode_text`, `decompressed_content`, `get_plain_content`,
`get_page_content`, `Document::load`, `load_mem`, `.decompress()`,
`IncrementalDocument`, `.save(` or `save_to(`. `tests_guard.rs` (GUARD-01) scans
`text_edit/` and every file whose first line is `//! offpdf:forbidden-api-scan`,
skipping comments and string literals. Production code has no
`unwrap`/`expect`/`panic!`/unchecked indexing on PDF-derived data (clippy deny
attributes in `text_edit/mod.rs`). Test seams exist only under `#[cfg(test)]`;
no environment variable or setting can weaken a check.

### Reason codes, engines, benchmarks and fixtures

Adding a reason code, the engine skip policy (`OFFPDF_REQUIRE_ENGINES=1`, no
`skip:` lines), BENCH and golden regeneration, and adding a font class or a
producer fixture: [`EDIT_MODEL.md`](../src/lib/editor/EDIT_MODEL.md#edit-text-contributor-workflow).

## 17. Manual test protocol

Reported in the PR; tick only what was actually run. Produce the files locally
and never commit them.

- [ ] For Word (Latin and Turkish; tagged, with bookmarks), LibreOffice, Google
      Docs, Chrome print, pdfTeX, XeLaTeX, InDesign (if available) and macOS
      Pages: a one-page PDF each.
- [ ] A two-column page (keyboard and VoiceOver reading order).
- [ ] A page with a stamp added in the same Save (qpdf's overlay wrapper path).
- [ ] Open Edit text; record editable and refused counts and the top reasons.
- [ ] Change three lines (shorter, same width, longer but fitting), including one
      style change; Save.
- [ ] Open the result in Acrobat Reader, macOS Preview, Chrome and Firefox; search
      finds the new words and not the old; compare at 400 %.
- [ ] Repeat on a rotated page and on a cropped page.
- [ ] Keyboard only: E → Tab → arrows → Enter → type → Tab → Bold → Enter → ⌘Z →
      ⌘⇧Z → Save.
- [ ] VoiceOver labels on lines, the editor, the format bar and the reason dialog.
- [ ] Light and dark themes at 1440 and 1024 px wide (screenshots attached).
