/**
 * Pure helpers for Edit text (SPEC §D.4): typing rules, the local width
 * estimate, caret geometry, problem messages and keyboard neighbours.
 *
 * Nothing here is authoritative: every commit is checked by Rust
 * (`preview_text_edits`) and again at Save. These helpers only drive the
 * editor's message row and width guide while typing.
 *
 * ## Width estimate (mirrored by Rust `fit::estimate_delta_pt`, golden-tested)
 *
 * For an editable run with metrics `m` and a requested style `s`:
 * - `tf  = s.sizePt` finite and `m.effectiveSize > 0` ? `m.tfSize × s.sizePt / m.effectiveSize` : `m.tfSize`
 * - `k   = m.hScale × m.textToUser` (0 when not finite)
 * - `tc  = s.letterSpacingPt` finite and `k > 0` ? `s.letterSpacingPt / k` : `m.charSpacing`
 * - surface = `m.surface` when the face is the run's own face, else `style.faces[face].surface`
 * - per character (code point) `ch`:
 *   - `ch === " "` in kern mode → `−m.kernSpace / 1000 × tf`
 *   - else the first surface font whose alphabet has `ch`: `w / 1000 × tf + tc` (+ `m.wordSpacing`
 *     when `ch === " "` and the font's `wordSpace`); no such font → `MISSING_GLYPH_EM × tf + tc`
 *   - clamped at ≥ 0
 * - `measure(text, s) = Σ advance × k` (points along the baseline)
 * - `estimateDeltaPt = measure(NFC(text), s) − measure(run.text, {})` — relative, so kerns kept by
 *   the minimal-diff rewrite cancel out; the estimated new extent is `m.originalWidth + delta`.
 * - `estimateCaretOffsets = [0, cumulative advances × k …]` (`chars + 1` entries).
 */

import type {
  SourceTextStyle,
  TextEditIn,
  TextEditProblemCode,
  TextEditVerdict,
  TextFace,
  TextFont,
  TextRect,
  TextRun,
  TextVec,
} from "../types";
import type { PageGeometry, SourceTextObject } from "./types";
import { displayedSize, makeMapping, pdfRectToViewport, pdfToViewport, viewportRectToPdf } from "./coords";
import { compactSourceTextStyle } from "./serialize";
import { problemCopy } from "./sourceTextCopy";

/** Same values as Rust `limits.rs`. */
export const STYLE_EPSILON = 0.001;
export const EDIT_TEXT_CHARS_MAX = 1000;
export const OVERLAP_WARN_TOL_PT = 0.5;
export const FIT_TOL_PT = 0.01;
export const SIZE_MIN_PT = 4;
export const SIZE_MAX_PT = 144;
export const SIZE_STEP_PT = 0.5;
export const LETTER_SPACING_MIN_PT = -2;
export const LETTER_SPACING_MAX_PT = 10;
export const LETTER_SPACING_STEP_PT = 0.1;
/** Advance (em) of a character no surface font can draw, in the estimate only. */
export const MISSING_GLYPH_EM = 0.5;
/** Lines moved by PageUp / PageDown in the text layer. */
export const PAGE_JUMP_LINES = 10;
/** Font size range of an Add text stamp (Rust clamps text stamps to the same range). */
export const STAMP_FONT_MIN_PT = 6;
export const STAMP_FONT_MAX_PT = 96;
const STAMP_DEFAULT_FONT_PT = 12;
/** A stamp box is at least one line tall and a few characters wide. */
const STAMP_LINE_HEIGHT_EM = 1.3;
const STAMP_MIN_WIDTH_EM = 4;
/** Width of the default box placed over a line that isn't level on screen. */
const STAMP_DEFAULT_WIDTH_EM = 10;
/** A baseline rising less than this per unit of run on screen (about 0.6°) is level. */
const LEVEL_SLOPE = 0.01;

/** The six inks of the colour popover (§A.7), as sent to Rust. */
export const TEXT_INKS = [
  { key: "black", hex: "#000000" },
  { key: "grey", hex: "#6b7380" },
  { key: "red", hex: "#c71c1c" },
  { key: "blue", hex: "#1c4fb8" },
  { key: "green", hex: "#14733d" },
  { key: "amber", hex: "#b55408" },
] as const;

const INVALID_CHAR = /[\u0000-\u001f\u007f-\u009f\u2028\u2029]/u;

/** Typed text as Rust receives it: NFC. */
export function normaliseTyped(text: string): string {
  return text.normalize("NFC");
}

export function fontsByKey(fonts: TextFont[]): Map<string, TextFont> {
  return new Map(fonts.map((f) => [f.key, f]));
}

function chars(text: string): string[] {
  return Array.from(text);
}

function currentFace(run: TextRun): TextFace {
  return run.style?.face ?? "regular";
}

/** Font keys that type `face` (the run's own surface when `face` is its current face). */
export function surfaceFor(run: TextRun, face?: TextFace): string[] {
  if (face === undefined || face === currentFace(run)) return run.metrics?.surface ?? [];
  const option = run.style?.faces[face];
  return option?.available ? option.surface : [];
}

interface GlyphHit {
  font: TextFont;
  width1000: number;
}

/** Character → index in `alphabet`, built once per font object. */
const alphabetIndex = new WeakMap<TextFont, Map<string, number>>();

function indexOfChar(font: TextFont, ch: string): number {
  let index = alphabetIndex.get(font);
  if (!index) {
    index = new Map();
    chars(font.alphabet).forEach((c, i) => {
      if (!index!.has(c)) index!.set(c, i);
    });
    alphabetIndex.set(font, index);
  }
  return index.get(ch) ?? -1;
}

function findGlyph(surface: string[], fonts: Map<string, TextFont>, ch: string): GlyphHit | null {
  for (const key of surface) {
    const font = fonts.get(key);
    if (!font) continue;
    const index = indexOfChar(font, ch);
    if (index < 0) continue;
    const width = font.widths[index];
    return { font, width1000: Number.isFinite(width) ? width : 0 };
  }
  return null;
}

/** Characters the surface can't draw: deduplicated, in typing order. In kern
 * mode a space is never "missing" (its placement is `spaceProblem`'s job). */
export function missingChars(run: TextRun, fonts: Map<string, TextFont>, text: string, face?: TextFace): string[] {
  const surface = surfaceFor(run, face);
  const kernSpaces = run.metrics?.spaceMode === "kern";
  const out: string[] = [];
  for (const ch of chars(normaliseTyped(text))) {
    if (out.includes(ch)) continue;
    if (ch === " " && kernSpaces) continue;
    if (!findGlyph(surface, fonts, ch)) out.push(ch);
  }
  return out;
}

/** Kern-mode rule (§A.3.4): a space must sit between two non-space characters and not be doubled. */
export function spaceProblem(run: TextRun, text: string): boolean {
  if (run.metrics?.spaceMode !== "kern") return false;
  const cs = chars(normaliseTyped(text));
  return cs.some((ch, i) => ch === " " && (i === 0 || i === cs.length - 1 || cs[i - 1] === " "));
}

interface Model {
  tf: number;
  tc: number;
  k: number;
  surface: string[];
}

function finite(n: number | undefined): n is number {
  return typeof n === "number" && Number.isFinite(n);
}

function modelFor(run: TextRun, style: SourceTextStyle): Model | null {
  const m = run.metrics;
  if (!m) return null;
  const k = finite(m.hScale * m.textToUser) ? m.hScale * m.textToUser : 0;
  const tf = finite(style.sizePt) && m.effectiveSize > 0 ? (m.tfSize * style.sizePt) / m.effectiveSize : m.tfSize;
  const tc = finite(style.letterSpacingPt) && k > 0 ? style.letterSpacingPt / k : m.charSpacing;
  return { tf, tc, k, surface: surfaceFor(run, style.face) };
}

function advances(run: TextRun, fonts: Map<string, TextFont>, text: string, style: SourceTextStyle): number[] {
  const model = modelFor(run, style);
  const m = run.metrics;
  if (!model || !m) return chars(text).map(() => 0);
  return chars(text).map((ch) => {
    let advance: number;
    if (ch === " " && m.spaceMode === "kern") {
      advance = (-m.kernSpace / 1000) * model.tf;
    } else {
      const hit = findGlyph(model.surface, fonts, ch);
      const em = hit ? hit.width1000 / 1000 : MISSING_GLYPH_EM;
      const wordSpace = ch === " " && hit?.font.wordSpace ? m.wordSpacing : 0;
      advance = em * model.tf + model.tc + wordSpace;
    }
    return finite(advance) ? Math.max(0, advance) * model.k : 0;
  });
}

function measure(run: TextRun, fonts: Map<string, TextFont>, text: string, style: SourceTextStyle): number {
  return advances(run, fonts, text, style).reduce((sum, a) => sum + a, 0);
}

/** Estimated width change in points (positive = wider). See the module doc. */
export function estimateDeltaPt(
  run: TextRun,
  fonts: Map<string, TextFont>,
  text: string,
  style: SourceTextStyle,
): number {
  if (!run.metrics) return 0;
  return measure(run, fonts, normaliseTyped(text), style) - measure(run, fonts, run.text, {});
}

/** `chars + 1` caret distances along `dir` for `text`, same model as `estimateDeltaPt`. */
export function estimateCaretOffsets(
  run: TextRun,
  fonts: Map<string, TextFont>,
  text: string,
  style: SourceTextStyle,
): number[] {
  const out = [0];
  let at = 0;
  for (const a of advances(run, fonts, normaliseTyped(text), style)) {
    at += a;
    out.push(at);
  }
  return out;
}

function sameNumber(a: number, b: number): boolean {
  return Math.abs(a - b) <= STYLE_EPSILON;
}

/** Drop fields equal to the run's current value (within 0.001) and unset ones (B7). */
export function normaliseStyle(run: TextRun, style: SourceTextStyle): SourceTextStyle {
  const out: SourceTextStyle = {};
  const m = run.metrics;
  const s = run.style;
  if (finite(style.sizePt) && !(m && sameNumber(style.sizePt, m.effectiveSize))) out.sizePt = style.sizePt;
  if (style.face !== undefined && style.face !== currentFace(run)) out.face = style.face;
  if (style.fill !== undefined) {
    const fill = style.fill.trim().toLowerCase();
    if (fill !== (s?.fill ?? "").toLowerCase()) out.fill = fill;
  }
  if (finite(style.letterSpacingPt) && !(m && sameNumber(style.letterSpacingPt, m.letterSpacingPt))) {
    out.letterSpacingPt = style.letterSpacingPt;
  }
  return out;
}

/** Same text (after NFC) and no style change: no object, no bytes. */
export function isNoOpEdit(run: TextRun, text: string, style: SourceTextStyle): boolean {
  return normaliseTyped(text) === run.text && Object.keys(normaliseStyle(run, style)).length === 0;
}

function verdict(run: TextRun, code: TextEditProblemCode, extra: Partial<TextEditVerdict> = {}): TextEditVerdict {
  return {
    runId: run.id,
    ok: false,
    code,
    chars: [],
    reason: null,
    face: null,
    field: null,
    detail: null,
    deltaPt: 0,
    newRect: null,
    caretOffsets: null,
    ...extra,
  };
}

/** New ink extent along the baseline, estimated. */
function estimatedExtent(run: TextRun, fonts: Map<string, TextFont>, text: string, style: SourceTextStyle): number {
  return (run.metrics?.originalWidth ?? 0) + estimateDeltaPt(run, fonts, text, style);
}

/**
 * The first local problem that blocks Done, as a verdict-shaped value for
 * `problemMessage` (same order as Rust `plan_page`), or null.
 */
export function blockingVerdict(
  run: TextRun,
  fonts: Map<string, TextFont>,
  text: string,
  style: SourceTextStyle,
): TextEditVerdict | null {
  if (!run.editable || !run.metrics || !run.style) {
    return verdict(run, "TEXT_EDIT_REFUSED", { reason: run.reason });
  }
  const typed = normaliseTyped(text);
  if (INVALID_CHAR.test(typed)) return verdict(run, "INVALID_TEXT");
  if (chars(typed).length > EDIT_TEXT_CHARS_MAX) return verdict(run, "TEXT_TOO_LONG");
  const s = normaliseStyle(run, style);
  if (typed === run.text && Object.keys(s).length === 0) return null;
  if (s.sizePt !== undefined && !run.style.sizeChangeable) return verdict(run, "STYLE_UNAVAILABLE", { field: "size" });
  if (s.face !== undefined && !run.style.sizeChangeable) return verdict(run, "STYLE_UNAVAILABLE", { field: "face" });
  if (s.fill !== undefined && !run.style.colourChangeable) return verdict(run, "STYLE_UNAVAILABLE", { field: "colour" });
  if (s.face !== undefined) {
    if (!run.style.faces[s.face].available) return verdict(run, "FACE_UNAVAILABLE", { face: s.face });
    const missingInFace = missingChars(run, fonts, typed, s.face);
    if (missingInFace.length > 0) return verdict(run, "FACE_UNAVAILABLE", { face: s.face, chars: missingInFace });
  }
  const missing = missingChars(run, fonts, typed, s.face);
  if (missing.length > 0) return verdict(run, "GLYPH_MISSING", { chars: missing });
  if (spaceProblem(run, typed)) return verdict(run, "SPACE_NOT_WRITABLE");
  const limit = Math.max(run.metrics.visibleExtent, run.metrics.originalWidth) + FIT_TOL_PT;
  if (estimatedExtent(run, fonts, typed, s) > limit) return verdict(run, "TEXT_OUTSIDE_VISIBLE_AREA");
  return null;
}

/** The code of `blockingVerdict`, or null. */
export function blockingProblem(
  run: TextRun,
  fonts: Map<string, TextFont>,
  text: string,
  style: SourceTextStyle,
): TextEditProblemCode | null {
  return blockingVerdict(run, fonts, text, style)?.code ?? null;
}

/** The new text would run into the next run on the line (warning only). */
export function overlapsNext(run: TextRun, fonts: Map<string, TextFont>, text: string, style: SourceTextStyle): boolean {
  const next = run.metrics?.nextObstacle;
  if (next === null || next === undefined) return false;
  return estimatedExtent(run, fonts, text, style) > next + OVERLAP_WARN_TOL_PT;
}

/** Index (0…chars) of the caret boundary nearest to `user` (unrotated user space). */
export function caretIndexAt(run: TextRun, user: TextVec, offsets: number[] = run.caretOffsets): number {
  if (offsets.length === 0) return 0;
  const along = (user.x - run.origin.x) * run.dir.x + (user.y - run.origin.y) * run.dir.y;
  if (!Number.isFinite(along)) return 0;
  let best = 0;
  for (let i = 1; i < offsets.length; i++) {
    if (Math.abs(offsets[i] - along) < Math.abs(offsets[best] - along)) best = i;
  }
  return best;
}

/** Hit box and caret offsets of an edited run: the committed verdict's while it
 * is current (same run, ok, offsets for this text), else the stored rect + estimate. */
export function editedGeometry(
  run: TextRun,
  obj: SourceTextObject,
  current: TextEditVerdict | null,
  fonts: Map<string, TextFont>,
): { rect: TextRect; caretOffsets: number[] } {
  const length = chars(obj.text).length + 1;
  if (
    current &&
    current.ok &&
    current.runId === obj.runId &&
    current.newRect &&
    current.caretOffsets &&
    current.caretOffsets.length === length
  ) {
    return { rect: { ...current.newRect }, caretOffsets: current.caretOffsets.slice() };
  }
  return { rect: { ...obj.rect }, caretOffsets: estimateCaretOffsets(run, fonts, obj.text, obj.style) };
}

/** The sentence for a failed verdict (§D.11.4); "" when the verdict is ok. Pass
 * `fonts` so GLYPH_MISSING can add the subset note, `fileName` for STALE. */
export function problemMessage(
  v: TextEditVerdict,
  run: TextRun,
  fonts?: Map<string, TextFont>,
  fileName?: string,
): string {
  if (v.ok || !v.code) return "";
  const subset = fonts ? surfaceFor(run, v.face ?? undefined).some((key) => fonts.get(key)?.subset === true) : false;
  return problemCopy(v.code, v.chars, {
    face: v.face,
    field: v.field,
    reason: v.reason ?? run.reason,
    subset,
    name: fileName,
  });
}

export type NeighbourKey = "up" | "down" | "left" | "right" | "home" | "end" | "pageUp" | "pageDown";

/** The run keyboard focus moves to from `currentId` (reading order from Rust's
 * `order`/`line`, never raw coordinates), or null when it would not move. */
export function readingNeighbour(runs: TextRun[], currentId: string, key: NeighbourKey): string | null {
  if (runs.length === 0) return null;
  const sorted = runs.slice().sort((a, b) => a.order - b.order);
  const pos = sorted.findIndex((r) => r.id === currentId);
  if (pos < 0) return key === "end" ? sorted[sorted.length - 1].id : sorted[0].id;
  const lines: TextRun[][] = [];
  for (const run of sorted) {
    const last = lines[lines.length - 1];
    if (last && last[0].line === run.line) last.push(run);
    else lines.push([run]);
  }
  const lineIdx = lines.findIndex((l) => l.some((r) => r.id === currentId));
  const inLine = lines[lineIdx].findIndex((r) => r.id === currentId);
  const toLine = (target: number): string | null => {
    const clamped = Math.min(lines.length - 1, Math.max(0, target));
    if (clamped === lineIdx) return null;
    const line = lines[clamped];
    return line[Math.min(inLine, line.length - 1)].id;
  };
  const toPos = (target: number): string | null =>
    target === pos || target < 0 || target >= sorted.length ? null : sorted[target].id;
  switch (key) {
    case "down":
      return toLine(lineIdx + 1);
    case "up":
      return toLine(lineIdx - 1);
    case "pageDown":
      return toLine(lineIdx + PAGE_JUMP_LINES);
    case "pageUp":
      return toLine(lineIdx - PAGE_JUMP_LINES);
    case "right":
      return toPos(pos + 1);
    case "left":
      return toPos(pos - 1);
    case "home":
      return toPos(0);
    case "end":
      return toPos(sorted.length - 1);
  }
}

/** The new text box "Add text here" adds: where it goes (PDF user space) and its font size. */
export interface TextStamp {
  rect: TextRect;
  fontSize: number;
}

/**
 * Box and size of an "Add text here" stamp over a refused line (§D.6.3), worked out on
 * screen so a `/Rotate` page gets the same shape. The size is the line's, clamped to the
 * stamp range in 0.5 pt steps (12 pt when unknown).
 * - A level line: its own box, grown to one line and four characters, top-left kept.
 * - A line turned, tilted or vertical on screen: its axis-aligned box can cover half the
 *   page (a 35° watermark), so it gets a default one-line box instead, upright, its first
 *   baseline at the line's start.
 * The box is kept on the page.
 */
export function textStampGeometry(run: TextRun, geometry: PageGeometry): TextStamp {
  const wanted = run.metrics?.effectiveSize ?? STAMP_DEFAULT_FONT_PT;
  const size = Number.isFinite(wanted)
    ? Math.min(STAMP_FONT_MAX_PT, Math.max(STAMP_FONT_MIN_PT, Math.round(wanted * 2) / 2))
    : STAMP_DEFAULT_FONT_PT;
  const page = displayedSize(geometry);
  const screen = makeMapping(geometry, page.w, page.h); // 1 unit = 1 pt as displayed, y down
  const start = pdfToViewport(run.origin, screen);
  const ahead = pdfToViewport({ x: run.origin.x + run.dir.x, y: run.origin.y + run.dir.y }, screen);
  const level = Math.abs(ahead.y - start.y) <= LEVEL_SLOPE * Math.abs(ahead.x - start.x);
  const lineH = size * STAMP_LINE_HEIGHT_EM;
  const box = level
    ? pdfRectToViewport(run.rect, screen)
    : { x: start.x, y: start.y - size, w: size * STAMP_DEFAULT_WIDTH_EM, h: lineH };
  const w = Math.min(Math.max(box.w, size * STAMP_MIN_WIDTH_EM), page.w);
  const h = Math.min(Math.max(box.h, lineH), page.h);
  const x = Math.min(Math.max(box.x, 0), page.w - w);
  const y = Math.min(Math.max(box.y, 0), page.h - h);
  return { rect: viewportRectToPdf({ x, y, w, h }, screen), fontSize: size };
}

/** The IPC shape of one committed change. */
export function toTextEditIn(obj: SourceTextObject): TextEditIn {
  return { runId: obj.runId, originalText: obj.originalText, text: obj.text, style: compactSourceTextStyle(obj.style) };
}

/** Order-independent key of a page's edits (preview cache key). */
export function editsSignature(objs: SourceTextObject[]): string {
  const items = objs
    .map((o) => ({ runId: o.runId, text: o.text, style: compactSourceTextStyle(o.style) }))
    .sort((a, b) => (a.runId < b.runId ? -1 : a.runId > b.runId ? 1 : 0));
  return JSON.stringify(items);
}
