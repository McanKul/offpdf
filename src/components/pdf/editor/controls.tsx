/**
 * Small form controls shared by the object inspector and the Edit text format
 * bar: a draft number field (commit on blur/Enter) and a colour field.
 */
import { useEffect, useRef, useState, type Ref } from "react";
import { toCssHex } from "@/lib/editor";
import { Icon } from "@/components/ui/Icon";

function formatNum(n: number): string {
  if (Number.isInteger(n)) return String(n);
  return String(Math.round(n * 1000) / 1000);
}

function parseDraft(s: string): number | null {
  const t = s.trim().replace(",", ".");
  if (t === "" || t === "-" || t === "." || t === "-.") return null;
  const n = Number(t);
  return Number.isFinite(n) ? n : null;
}

/** Word/Paint-style number field: empty while typing, commit on blur/Enter. */
export function DraftNumber({
  label,
  hideLabel,
  inline,
  value,
  min,
  max,
  suffix,
  onCommit,
  tabIndex,
  inputRef,
  rovingKey,
  onDone,
  live,
}: {
  label: string;
  hideLabel?: boolean;
  /** W/H row: label | input | suffix as sibling grid cells. */
  inline?: boolean;
  value: number;
  min: number;
  max: number;
  suffix?: string;
  onCommit: (n: number) => void;
  tabIndex?: number;
  inputRef?: Ref<HTMLInputElement>;
  /** Marks the input as an item of a roving-tabindex toolbar. */
  rovingKey?: string;
  /** Called after Enter (commit) or Esc (revert) left the field. */
  onDone?: () => void;
  /** Also commit every in-range value while typing (the field may never blur before use). */
  live?: boolean;
}) {
  const [focused, setFocused] = useState(false);
  const [draft, setDraft] = useState(formatNum(value));
  const reverting = useRef(false);
  useEffect(() => {
    if (!focused) setDraft(formatNum(value));
  }, [value, focused]);

  const commit = (raw: string) => {
    const n = parseDraft(raw);
    if (n == null) {
      setDraft(formatNum(value));
      return;
    }
    const clamped = Math.min(max, Math.max(min, n));
    onCommit(clamped);
    setDraft(formatNum(clamped));
  };

  const input = (
    <input
      ref={inputRef}
      className="pdf-editor__num-input"
      inputMode="decimal"
      value={draft}
      aria-label={label}
      tabIndex={tabIndex}
      data-roving={rovingKey}
      onFocus={() => setFocused(true)}
      onChange={(e) => {
        setDraft(e.target.value);
        const n = live ? parseDraft(e.target.value) : null;
        if (n !== null && n >= min && n <= max) onCommit(n);
      }}
      onBlur={(e) => {
        setFocused(false);
        if (reverting.current) {
          reverting.current = false;
          setDraft(formatNum(value));
          return;
        }
        commit(e.target.value);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          (e.target as HTMLInputElement).blur();
          onDone?.();
        }
        if (e.key === "Escape") {
          // Revert without committing what was typed.
          e.preventDefault();
          e.stopPropagation();
          reverting.current = true;
          (e.target as HTMLInputElement).blur();
          onDone?.();
        }
      }}
    />
  );
  const suf = suffix ? (
    <span className="pdf-editor__num-suffix" aria-hidden>
      {suffix}
    </span>
  ) : null;

  if (inline) {
    return (
      <>
        <span className="pdf-editor__wh-key">{label}</span>
        {input}
        {suf}
      </>
    );
  }

  return (
    <div className="pdf-editor__num">
      {!hideLabel && <label className="field__label">{label}</label>}
      <div className="pdf-editor__num-row">
        {input}
        {suf}
      </div>
    </div>
  );
}

/** Colour picker + hex field + preset swatches (+ optional eyedropper). */
export function ColorField({
  label,
  icon,
  value,
  presets,
  fallback,
  active,
  onChange,
  onPickFromPage,
}: {
  label: string;
  icon?: "droplet" | "square" | "squareFill";
  value: string;
  /** Swatch colours, `#rrggbb`. */
  presets: readonly string[];
  /** Shown when `value` is not a readable colour. */
  fallback: string;
  active?: boolean;
  onChange: (hex: string) => void;
  onPickFromPage?: () => void;
}) {
  const hex = toCssHex(value, fallback);
  const [typed, setTyped] = useState(hex);
  useEffect(() => {
    setTyped(hex);
  }, [hex]);
  return (
    <div className="pdf-editor__color">
      <div className="pdf-editor__color-row">
        {icon ? (
          <span className="pdf-editor__color-kind" title={label} aria-hidden>
            <Icon name={icon} size={15} />
          </span>
        ) : (
          <label className="field__label">{label}</label>
        )}
        <input
          type="color"
          aria-label={label}
          value={hex}
          onChange={(e) => onChange(e.target.value)}
          className="pdf-editor__color-input"
        />
        <input
          className="pdf-editor__hex"
          value={typed}
          spellCheck={false}
          aria-label={`${label} hex`}
          onChange={(e) => {
            const v = e.target.value.trim();
            setTyped(v.startsWith("#") || v.length === 0 ? v : `#${v}`);
            const next = v.startsWith("#") ? v : `#${v}`;
            if (/^#[0-9a-fA-F]{6}$/.test(next)) onChange(next.toLowerCase());
            else if (/^#[0-9a-fA-F]{3}$/.test(next)) onChange(toCssHex(next));
          }}
        />
        {onPickFromPage && (
          <button
            type="button"
            className={`pdf-editor__eyedrop${active ? " is-active" : ""}`}
            onClick={onPickFromPage}
            title={active ? "Click the page to sample a color" : "Pick color from the page"}
            aria-label={active ? "Click the page to sample a color" : "Pick color from the page"}
            aria-pressed={active}
          >
            <Icon name="eyedropper" size={16} />
          </button>
        )}
      </div>
      <div className="pdf-editor__swatches">
        {presets.map((c) => (
          <button
            key={c}
            type="button"
            title={c}
            className={`pdf-editor__swatch${hex === c ? " is-active" : ""}`}
            style={{ background: c }}
            onClick={() => onChange(c)}
          />
        ))}
      </div>
    </div>
  );
}
