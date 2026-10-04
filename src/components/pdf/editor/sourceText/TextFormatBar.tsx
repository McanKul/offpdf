/**
 * Format bar of the inline editor (SPEC §D.6.2): size, bold/italic, colour,
 * letter spacing, the subset's letters, Restore original / Cancel / Done.
 * A roving-tabindex toolbar (←/→ move, Enter/Space activate, Esc/Shift+Tab back
 * to the input). Buttons never take focus from the input on mouse down. An
 * unavailable control stays focusable (`aria-disabled`) and, when activated,
 * writes its exact reason into the editor's message row (B8, B9).
 */
import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type MouseEvent } from "react";
import { Icon } from "@/components/ui/Icon";
import {
  LETTER_SPACING_MAX_PT,
  LETTER_SPACING_MIN_PT,
  LETTER_SPACING_STEP_PT,
  SIZE_MAX_PT,
  SIZE_MIN_PT,
  SIZE_STEP_PT,
  TEXT_INKS,
  surfaceFor,
} from "@/lib/editor";
import { UI, charList, problemCopy } from "@/lib/editor/sourceTextCopy";
import type { SourceTextStyle, TextFace, TextFont, TextRun } from "@/lib/types";
import { ColorField, DraftNumber } from "../controls";

/** A style edit, or the sentence saying why it is not available. */
export interface StyleChange {
  style?: SourceTextStyle;
  notice?: string;
}

const round2 = (n: number) => Math.round(n * 100) / 100;
/** Space between the bar (or what it must clear) and an open popover, CSS px. */
const POP_GAP_PX = 4;
const MIN_FONT_PX = 12;
const FAMILY: Record<string, string> = {
  serif: '"Times New Roman", "Liberation Serif", Georgia, serif',
  sans: 'Arial, "Liberation Sans", "Helvetica Neue", sans-serif',
  mono: '"Courier New", "Liberation Mono", monospace',
};
const clamp = (n: number, min: number, max: number) => Math.min(max, Math.max(min, n));
const keep = (e: MouseEvent) => e.preventDefault();

export function currentFace(run: TextRun, style: SourceTextStyle): TextFace {
  return style.face ?? run.style?.face ?? "regular";
}

function flags(face: TextFace): { bold: boolean; italic: boolean } {
  return { bold: face === "bold" || face === "boldItalic", italic: face === "italic" || face === "boldItalic" };
}

function faceOf(bold: boolean, italic: boolean): TextFace {
  if (bold) return italic ? "boldItalic" : "bold";
  return italic ? "italic" : "regular";
}

/** Why `run` can't switch to `face`, or null (its own face is always fine). */
function faceProblem(run: TextRun, face: TextFace): string | null {
  if (!run.style || face === run.style.face) return null;
  if (!run.style.sizeChangeable) return problemCopy("STYLE_UNAVAILABLE", [], { field: "face" });
  if (!run.style.faces[face].available) return problemCopy("FACE_UNAVAILABLE", [], { face });
  return null;
}

function toggleTarget(run: TextRun, style: SourceTextStyle, which: "bold" | "italic"): TextFace {
  const f = flags(currentFace(run, style));
  return which === "bold" ? faceOf(!f.bold, f.italic) : faceOf(f.bold, !f.italic);
}

/** Bold or italic toggled (⌘B / ⌘I and the B / I buttons). */
export function toggledFace(run: TextRun, style: SourceTextStyle, which: "bold" | "italic"): StyleChange {
  const target = toggleTarget(run, style, which);
  const problem = faceProblem(run, target);
  return problem ? { notice: problem } : { style: { ...style, face: target } };
}

/** Size stepped by `delta` effective points (⌘⇧. / ⌘⇧, and Smaller / Larger). */
export function steppedSize(run: TextRun, style: SourceTextStyle, delta: number): StyleChange {
  if (!run.metrics || !run.style?.sizeChangeable) return { notice: problemCopy("STYLE_UNAVAILABLE", [], { field: "size" }) };
  const now = style.sizePt ?? run.metrics.effectiveSize;
  return { style: { ...style, sizePt: round2(clamp(now + delta, SIZE_MIN_PT, SIZE_MAX_PT)) } };
}

/**
 * How the inline input draws the draft (§D.6.1): family by the font's hint, the
 * draft's face, size (never below 12 px; `floored` says it is shown larger),
 * letter spacing and colour (the run's fill unless changed).
 */
export function chipFont(
  run: TextRun,
  fonts: Map<string, TextFont>,
  style: SourceTextStyle,
  pxPerPt: number,
): { css: CSSProperties; floored: boolean } {
  const metrics = run.metrics;
  const face = currentFace(run, style);
  const sizePx = (style.sizePt ?? metrics?.effectiveSize ?? MIN_FONT_PX) * pxPerPt;
  const fill = style.fill ?? run.style?.fill;
  return {
    floored: sizePx < MIN_FONT_PX,
    css: {
      fontFamily: FAMILY[fonts.get(metrics?.surface[0] ?? "")?.familyHint ?? "sans"],
      fontSize: Math.max(MIN_FONT_PX, sizePx),
      fontWeight: flags(face).bold ? 700 : 400,
      fontStyle: flags(face).italic ? "italic" : "normal",
      letterSpacing: (style.letterSpacingPt ?? metrics?.letterSpacingPt ?? 0) * pxPerPt,
      color: fill ?? undefined,
    },
  };
}

/** Space between the top of the visible stage and `el`, or Infinity outside a stage. */
function roomAbove(el: HTMLElement | null): number {
  const stage = el?.closest(".pdf-editor__stage");
  return el && stage ? el.getBoundingClientRect().top - stage.getBoundingClientRect().top : Infinity;
}

/** How far below the bar's bottom the line being edited and its message row reach (CSS px, ≥ 0). */
function editorBelow(bar: HTMLElement): number {
  const editor = bar.closest(".st-editor");
  if (!editor) return 0;
  const bottom = bar.getBoundingClientRect().bottom;
  const reach = [...editor.querySelectorAll(".st-chip, .st-msg")].map((el) => el.getBoundingClientRect().bottom - bottom);
  return Math.ceil(Math.max(0, ...reach));
}

function without(style: SourceTextStyle, key: keyof SourceTextStyle): SourceTextStyle {
  const { [key]: _dropped, ...rest } = style;
  return rest;
}

/** Every character the face can type, in font order, space included once. */
function alphabetOf(run: TextRun, fonts: Map<string, TextFont>, face: TextFace): string[] {
  const seen = new Set<string>();
  for (const key of surfaceFor(run, face)) for (const ch of Array.from(fonts.get(key)?.alphabet ?? "")) seen.add(ch);
  return [...seen];
}

type Pop = "colour" | "spacing" | "letters";

export interface TextFormatBarProps {
  run: TextRun;
  fonts: Map<string, TextFont>;
  style: SourceTextStyle;
  onStyle: (next: SourceTextStyle) => void;
  onNotice: (sentence: string) => void;
  canRestore: boolean;
  onRestore: () => void;
  onCancel: () => void;
  onDone: () => void;
  onBackToInput: () => void;
  placement: "above" | "below";
}

export function TextFormatBar(props: TextFormatBarProps) {
  const { run, fonts, style, onStyle, onNotice } = props;
  const rootRef = useRef<HTMLDivElement>(null);
  const [active, setActive] = useState("smaller");
  const popRef = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState<{ pop: Pop; byKey: boolean } | null>(null);
  /** Where the open popover sits: above the bar, or below it and clear of the line being edited. */
  const [place, setPlace] = useState({ up: false, drop: 0 });
  const [custom, setCustom] = useState(false);
  const metrics = run.metrics;
  const runStyle = run.style;

  useEffect(() => {
    if (open?.byKey) rootRef.current?.querySelector<HTMLElement>(".st-bar__pop button, .st-bar__pop input")?.focus();
  }, [open]);

  // Measured after every render while open (its height and the message row can change): up only from a
  // bar above the line with room for it in the visible stage; otherwise down, below the line and its
  // message row, so the popover never covers the text being edited.
  useLayoutEffect(() => {
    const bar = rootRef.current;
    const pop = popRef.current;
    if (!open || !bar || !pop) return;
    const up = props.placement === "above" && roomAbove(bar) >= pop.offsetHeight + POP_GAP_PX;
    const drop = up ? 0 : editorBelow(bar);
    if (up !== place.up || drop !== place.drop) setPlace({ up, drop });
  });

  if (!metrics || !runStyle) return null;
  const face = currentFace(run, style);
  const f = flags(face);
  const size = style.sizePt ?? metrics.effectiveSize;
  const spacing = style.letterSpacingPt ?? metrics.letterSpacingPt;
  const fill = style.fill ?? runStyle.fill;
  const primary = fonts.get(metrics.surface[0] ?? "");
  const showLetters = !!primary && primary.embedded && primary.subset;
  const keys = ["smaller", ...(runStyle.sizeChangeable ? ["size"] : []), "larger", "bold", "italic", "colour", "spacing"]
    .concat(showLetters ? ["letters"] : [], props.canRestore ? ["restore"] : [], ["cancel", "done"]);
  const rovingKey = keys.includes(active) ? active : keys[0];

  const focusKey = (key: string) => {
    rootRef.current?.querySelector<HTMLElement>(`[data-roving="${key}"]`)?.focus();
    setActive(key);
  };
  const item = (key: string) => ({ "data-roving": key, tabIndex: key === rovingKey ? 0 : -1, onMouseDown: keep });
  const apply = (change: StyleChange) => (change.style ? onStyle(change.style) : change.notice && onNotice(change.notice));
  const toggle = (pop: Pop, e: MouseEvent) => setOpen((cur) => (cur?.pop === pop ? null : { pop, byKey: e.detail === 0 }));
  const popProps = (extra = "") => ({
    ref: popRef,
    className: `st-bar__pop${extra}${place.up ? " is-up" : ""}`,
    style: !place.up && place.drop > 0 ? { top: `calc(100% + ${place.drop + POP_GAP_PX}px)` } : undefined,
  });
  const faceTitle = (which: "bold" | "italic") => faceProblem(run, toggleTarget(run, style, which));
  const sizeProblem = runStyle.sizeChangeable ? null : problemCopy("STYLE_UNAVAILABLE", [], { field: "size" });
  const colourProblem = runStyle.colourChangeable ? null : problemCopy("STYLE_UNAVAILABLE", [], { field: "colour" });

  const spacingStep = (sign: -1 | 1) => (
    <button type="button" className="st-bar__btn" onMouseDown={keep}
      aria-label={`${UI.bar.letterSpacing} ${sign > 0 ? "+" : "−"}${LETTER_SPACING_STEP_PT} pt`}
      onClick={() => onStyle({
        ...style,
        letterSpacingPt: round2(clamp(spacing + sign * LETTER_SPACING_STEP_PT, LETTER_SPACING_MIN_PT, LETTER_SPACING_MAX_PT)),
      })}>
      <Icon name={sign > 0 ? "plus" : "minus"} size={14} />
    </button>
  );

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const target = e.target as HTMLElement;
    const inPop = !!target.closest(".st-bar__pop");
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      if (open) {
        const trigger = open.pop;
        setOpen(null);
        focusKey(trigger);
      } else props.onBackToInput();
      return;
    }
    if (e.key === "Tab" && e.shiftKey && !inPop) {
      e.preventDefault();
      props.onBackToInput();
      return;
    }
    if ((e.key !== "ArrowLeft" && e.key !== "ArrowRight") || inPop) return;
    if (target instanceof HTMLInputElement) {
      const atEdge = e.key === "ArrowLeft" ? target.selectionStart === 0 : target.selectionEnd === target.value.length;
      if (!atEdge) return;
    }
    e.preventDefault();
    const at = keys.indexOf(rovingKey);
    focusKey(keys[(at + (e.key === "ArrowRight" ? 1 : keys.length - 1)) % keys.length]);
  };

  return (
    <div
      ref={rootRef}
      className={`st-bar st-bar--${props.placement}`}
      role="toolbar"
      aria-label={UI.bar.label}
      onKeyDown={onKeyDown}
      onFocus={(e) => {
        const key = (e.target as HTMLElement).dataset?.roving;
        if (key) setActive(key);
      }}
    >
      <button type="button" {...item("smaller")} className="st-bar__btn" aria-label={UI.bar.smaller} title={sizeProblem ?? UI.bar.smaller}
        aria-disabled={sizeProblem ? true : undefined} onClick={() => apply(steppedSize(run, style, -SIZE_STEP_PT))}>
        <Icon name="minus" size={14} />
      </button>
      {runStyle.sizeChangeable ? (
        <span className="st-bar__size">
          <DraftNumber label={UI.bar.fontSize} hideLabel value={round2(size)} min={SIZE_MIN_PT} max={SIZE_MAX_PT} suffix="pt"
            rovingKey="size" tabIndex={rovingKey === "size" ? 0 : -1} onDone={() => focusKey("size")} live
            onCommit={(n) => onStyle({ ...style, sizePt: n })} />
        </span>
      ) : (
        <span className="st-bar__size-text">{round2(size)} pt</span>
      )}
      <button type="button" {...item("larger")} className="st-bar__btn" aria-label={UI.bar.larger} title={sizeProblem ?? UI.bar.larger}
        aria-disabled={sizeProblem ? true : undefined} onClick={() => apply(steppedSize(run, style, SIZE_STEP_PT))}>
        <Icon name="plus" size={14} />
      </button>
      <span className="st-bar__sep" aria-hidden />
      {(["bold", "italic"] as const).map((which) => {
        const problem = faceTitle(which);
        return (
          <button key={which} type="button" {...item(which)} className={`st-bar__btn st-bar__btn--${which}`}
            aria-label={UI.bar[which]} title={problem ?? UI.bar[which]} aria-pressed={f[which]}
            aria-disabled={problem ? true : undefined} onClick={() => apply(toggledFace(run, style, which))}>
            {which === "bold" ? "B" : "I"}
          </button>
        );
      })}
      <button type="button" {...item("colour")} className="st-bar__btn" aria-label={UI.bar.textColour} title={colourProblem ?? UI.bar.textColour}
        aria-disabled={colourProblem ? true : undefined} aria-expanded={open?.pop === "colour"}
        onClick={(e) => (colourProblem ? onNotice(colourProblem) : toggle("colour", e))}>
        <span className="st-bar__dot" style={fill ? { background: fill } : undefined} aria-hidden />
        <Icon name="chevronDown" size={12} />
      </button>
      <button type="button" {...item("spacing")} className="st-bar__btn" aria-label={UI.bar.letterSpacing} title={UI.bar.letterSpacing}
        aria-expanded={open?.pop === "spacing"} onClick={(e) => toggle("spacing", e)}>
        <span aria-hidden>↔</span>
        <Icon name="chevronDown" size={12} />
      </button>
      {showLetters && (
        <button type="button" {...item("letters")} className="st-bar__btn st-bar__btn--text" aria-expanded={open?.pop === "letters"}
          onClick={(e) => toggle("letters", e)}>
          {UI.bar.letters}
        </button>
      )}
      <span className="st-bar__sep" aria-hidden />
      {props.canRestore && (
        <button type="button" {...item("restore")} className="st-bar__btn st-bar__btn--text" onClick={props.onRestore}>
          {UI.bar.restore}
        </button>
      )}
      <button type="button" {...item("cancel")} className="st-bar__btn st-bar__btn--text" onClick={props.onCancel}>
        {UI.bar.cancel}
      </button>
      <button type="button" {...item("done")} className="st-bar__btn st-bar__btn--done" onClick={props.onDone}>
        <Icon name="check" size={14} />
        {UI.bar.done}
      </button>

      {open?.pop === "colour" && (
        <div {...popProps()} role="group" aria-label={UI.bar.textColour}>
          <div className="st-bar__inks">
            <button type="button" className="st-bar__btn st-bar__btn--text" aria-pressed={style.fill === undefined}
              onMouseDown={keep} onClick={() => onStyle(without(style, "fill"))}>
              {UI.bar.originalColour}
            </button>
            {TEXT_INKS.map((ink) => (
              <button key={ink.key} type="button" className="st-bar__ink" style={{ background: ink.hex }} aria-label={UI.bar[ink.key]}
                title={UI.bar[ink.key]} aria-pressed={style.fill === ink.hex} onMouseDown={keep}
                onClick={() => onStyle({ ...style, fill: ink.hex })} />
            ))}
            <button type="button" className="st-bar__btn st-bar__btn--text" aria-expanded={custom} onMouseDown={keep}
              onClick={() => setCustom((c) => !c)}>
              {UI.bar.customColour}
            </button>
          </div>
          {custom && (
            <ColorField label={UI.bar.customColour} value={fill ?? TEXT_INKS[0].hex} presets={TEXT_INKS.map((i) => i.hex)}
              fallback={TEXT_INKS[0].hex} onChange={(hex) => onStyle({ ...style, fill: hex })} />
          )}
        </div>
      )}
      {open?.pop === "spacing" && (
        <div {...popProps(" st-bar__spacing")} role="group" aria-label={UI.bar.letterSpacing}>
          {spacingStep(-1)}
          <DraftNumber label={UI.bar.letterSpacing} hideLabel value={round2(spacing)} min={LETTER_SPACING_MIN_PT}
            max={LETTER_SPACING_MAX_PT} suffix="pt" live onCommit={(n) => onStyle({ ...style, letterSpacingPt: n })} />
          {spacingStep(1)}
        </div>
      )}
      {open?.pop === "letters" && (
        <div {...popProps(" st-bar__letters")} role="group" aria-label={UI.bar.lettersTitle}>
          <div className="st-bar__pop-title">{UI.bar.lettersTitle}</div>
          <p className="st-bar__alphabet">{charList(alphabetOf(run, fonts, face))}</p>
        </div>
      )}
    </div>
  );
}
