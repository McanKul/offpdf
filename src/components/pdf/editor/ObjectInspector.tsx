import { useEffect, useState } from "react";
import type { EditObject, LayerDir } from "@/lib/editor";
import { isClosedShapeObject, isMarkupObject, isNoneFill, sizeWithAspect, toCssHex } from "@/lib/editor";
import { Icon } from "@/components/ui/Icon";

const PRESETS = [
  "#111827",
  "#ffffff",
  "#dc2626",
  "#16a34a",
  "#2563eb",
  "#6b7280",
];

export type ColorPickTarget = "color" | "fill" | "stroke";

export function ObjectInspector({
  obj,
  picking,
  layerIndex,
  layerCount,
  pageCount,
  onChange,
  onPickFromPage,
  onReorder,
}: {
  obj: EditObject;
  picking?: ColorPickTarget | null;
  /** 1-based, back = 1, front = layerCount */
  layerIndex: number;
  layerCount: number;
  /** Assembled page count (GoTo dest picker). */
  pageCount?: number;
  onChange: (patch: Partial<EditObject>) => void;
  onPickFromPage?: (target: ColorPickTarget) => void;
  onReorder?: (dir: LayerDir) => void;
}) {
  const opacity = "opacity" in obj && typeof obj.opacity === "number" ? obj.opacity : 1;
  const shape = isClosedShapeObject(obj) ? obj : null;
  const filled = !!shape && !isNoneFill(shape.fill);
  const markup = isMarkupObject(obj);
  const hasBox = obj.kind !== "line" && obj.kind !== "ink" && obj.kind !== "markupInk";
  const canLockAspect = !!shape || obj.kind === "image";
  const aspectOn = obj.kind === "image" ? obj.keepAspect !== false : !!obj.keepAspect;

  return (
    <div className="pdf-editor__inspector">
      {obj.kind === "link" && (
        <>
          <div className="pdf-editor__icon-row" role="group" aria-label="Link type">
            <button
              type="button"
              className={`btn btn--sm ${obj.action.type === "uri" ? "btn--primary" : "btn--ghost"}`}
              onClick={() =>
                onChange({
                  action: {
                    type: "uri",
                    uri: obj.action.type === "uri" ? obj.action.uri : "https://",
                  },
                } as Partial<EditObject>)
              }
            >
              URL
            </button>
            <button
              type="button"
              className={`btn btn--sm ${obj.action.type === "goto" ? "btn--primary" : "btn--ghost"}`}
              onClick={() =>
                onChange({
                  action: {
                    type: "goto",
                    destPageIndex: obj.action.type === "goto" ? obj.action.destPageIndex : 0,
                  },
                } as Partial<EditObject>)
              }
            >
              Page
            </button>
          </div>
          {obj.action.type === "uri" && (
            <>
              <label className="field__label">Address</label>
              <input
                className="pdf-editor__inspector-text"
                type="text"
                spellCheck={false}
                value={obj.action.uri}
                onChange={(e) =>
                  onChange({ action: { type: "uri", uri: e.target.value } } as Partial<EditObject>)
                }
              />
            </>
          )}
          {obj.action.type === "goto" && (
            <DraftNumber
              label="Dest page"
              value={obj.action.destPageIndex + 1}
              min={1}
              max={Math.max(1, pageCount ?? obj.action.destPageIndex + 1)}
              onCommit={(n) =>
                onChange({
                  action: { type: "goto", destPageIndex: n - 1 },
                } as Partial<EditObject>)
              }
            />
          )}
        </>
      )}
      {markup && (
        <>
          <label className="field__label">Author</label>
          <input
            className="input"
            type="text"
            value={obj.author}
            placeholder="Author"
            onChange={(e) => onChange({ author: e.target.value } as Partial<EditObject>)}
          />
          <label className="field__label">Comment</label>
          <textarea
            className="pdf-editor__inspector-text"
            rows={2}
            value={obj.comment ?? ""}
            onChange={(e) => onChange({ comment: e.target.value || undefined } as Partial<EditObject>)}
          />
          <ColorField
            label="Color"
            icon="droplet"
            value={toCssHex(obj.color, "#facc15")}
            active={picking === "color"}
            onChange={(hex) => onChange({ color: hex } as Partial<EditObject>)}
            onPickFromPage={onPickFromPage ? () => onPickFromPage("color") : undefined}
          />
        </>
      )}

      {obj.kind === "text" && (
        <>
          <label className="field__label">Text</label>
          <textarea
            className="pdf-editor__inspector-text"
            rows={3}
            value={obj.content}
            onChange={(e) => onChange({ content: e.target.value } as Partial<EditObject>)}
          />
          <DraftNumber
            label="Size"
            value={obj.fontSize}
            min={8}
            max={96}
            suffix="pt"
            onCommit={(n) => onChange({ fontSize: n } as Partial<EditObject>)}
          />
          <div className="pdf-editor__icon-row" role="group" aria-label="Align">
            {([
              ["left", "alignLeft", "Align left"],
              ["center", "alignCenter", "Align center"],
              ["right", "alignRight", "Align right"],
            ] as const).map(([a, icon, title]) => (
              <button
                key={a}
                type="button"
                title={title}
                aria-label={title}
                className={`btn btn--sm ${obj.align === a ? "btn--primary" : "btn--ghost"}`}
                onClick={() => onChange({ align: a } as Partial<EditObject>)}
              >
                <Icon name={icon} size={15} />
              </button>
            ))}
          </div>
          <ColorField
            label="Color"
            icon="droplet"
            value={obj.color ?? "#111827"}
            active={picking === "color"}
            onChange={(hex) => onChange({ color: hex } as Partial<EditObject>)}
            onPickFromPage={onPickFromPage ? () => onPickFromPage("color") : undefined}
          />
        </>
      )}

      {obj.kind === "redact" && (
        <>
          <ColorField
            label="Fill"
            icon="squareFill"
            value={toCssHex(obj.fill, "#000000")}
            active={picking === "fill"}
            onChange={(hex) => onChange({ fill: hex } as Partial<EditObject>)}
            onPickFromPage={onPickFromPage ? () => onPickFromPage("fill") : undefined}
          />
          <label className="field__label">Label</label>
          <input
            className="pdf-editor__inspector-text"
            type="text"
            value={obj.label ?? ""}
            placeholder="Optional (e.g. REDACTED)"
            onChange={(e) =>
              onChange({ label: e.target.value.trim() ? e.target.value : undefined } as Partial<EditObject>)
            }
          />
        </>
      )}

      {shape && (
        <>
          <button
            type="button"
            className={`pdf-editor__toggle${filled ? " is-on" : ""}`}
            title={filled ? "Fill on — click for border only" : "Border only — click to fill"}
            aria-pressed={filled}
            onClick={() =>
              onChange({
                fill: filled ? "none" : toCssHex(shape.fill, "#111827"),
              } as Partial<EditObject>)
            }
          >
            <Icon name={filled ? "squareFill" : "square"} size={16} />
            <span>Fill inside</span>
          </button>
          {filled && (
            <ColorField
              label="Fill"
              icon="squareFill"
              value={toCssHex(shape.fill, "#111827")}
              active={picking === "fill"}
              onChange={(hex) => onChange({ fill: hex } as Partial<EditObject>)}
              onPickFromPage={onPickFromPage ? () => onPickFromPage("fill") : undefined}
            />
          )}
          <ColorField
            label="Border"
            icon="square"
            value={shape.stroke ?? "#111827"}
            active={picking === "stroke"}
            onChange={(hex) => onChange({ stroke: hex } as Partial<EditObject>)}
            onPickFromPage={onPickFromPage ? () => onPickFromPage("stroke") : undefined}
          />
        </>
      )}

      {(obj.kind === "line" || obj.kind === "ink") && (
        <ColorField
          label="Color"
          icon="droplet"
          value={obj.stroke ?? "#111827"}
          active={picking === "stroke"}
          onChange={(hex) => onChange({ stroke: hex } as Partial<EditObject>)}
          onPickFromPage={onPickFromPage ? () => onPickFromPage("stroke") : undefined}
        />
      )}

      {(shape || obj.kind === "line" || obj.kind === "ink") && (
        <DraftNumber
          label="Line width"
          value={(shape ? shape.strokeWidth : obj.kind === "line" || obj.kind === "ink" ? obj.strokeWidth : undefined) ?? 1.5}
          min={0.5}
          max={24}
          suffix="pt"
          onCommit={(n) => onChange({ strokeWidth: n } as Partial<EditObject>)}
        />
      )}

      {hasBox && (
        <div className="pdf-editor__wh">
          <div className="pdf-editor__wh-row">
            <DraftNumber
              label="W"
              inline
              value={Math.round(obj.rect.w * 10) / 10}
              min={4}
              max={4000}
              suffix="pt"
              onCommit={(n) => onChange({ rect: sizeWithAspect(obj.rect, { w: n }, canLockAspect && aspectOn) })}
            />
          </div>
          {canLockAspect && (
            <button
              type="button"
              className={`pdf-editor__wh-lock${aspectOn ? " is-on" : ""}`}
              title={aspectOn ? "Aspect ratio locked — click to unlock" : "Aspect ratio free — click to lock"}
              aria-label={aspectOn ? "Unlock aspect ratio" : "Lock aspect ratio"}
              aria-pressed={aspectOn}
              onClick={() => onChange({ keepAspect: !aspectOn })}
            >
              <Icon name="lock" size={15} />
            </button>
          )}
          <div className="pdf-editor__wh-row">
            <DraftNumber
              label="H"
              inline
              value={Math.round(obj.rect.h * 10) / 10}
              min={4}
              max={4000}
              suffix="pt"
              onCommit={(n) => onChange({ rect: sizeWithAspect(obj.rect, { h: n }, canLockAspect && aspectOn) })}
            />
          </div>
        </div>
      )}

      {obj.kind !== "link" && obj.kind !== "redact" && !markup && (
        <DraftNumber
          label="Rotation"
          value={obj.objectRotate ?? 0}
          min={-180}
          max={180}
          suffix="°"
          onCommit={(n) => onChange({ objectRotate: n })}
        />
      )}

      {obj.kind !== "link" && !markup && <label className="field__label">Opacity</label>}
      {obj.kind !== "link" && !markup && (
        <div className="pdf-editor__opacity">
          <input
            type="range"
            min={0.1}
            max={1}
            step={0.05}
            value={opacity}
            aria-label="Opacity"
            onChange={(e) => onChange({ opacity: Number(e.target.value) } as Partial<EditObject>)}
          />
          <DraftNumber
            label="Opacity percent"
            hideLabel
            value={Math.round(opacity * 100)}
            min={10}
            max={100}
            suffix="%"
            onCommit={(n) => onChange({ opacity: n / 100 } as Partial<EditObject>)}
          />
        </div>
      )}

      {onReorder && layerCount > 0 && obj.kind !== "link" && !markup && (
        <div className="pdf-editor__layer">
          <label className="field__label">Layer</label>
          <div className="muted" style={{ fontSize: 12 }}>
            {layerIndex}/{layerCount}
          </div>
          <div className="pdf-editor__layer-btns">
            <button type="button" className="btn btn--ghost btn--sm" title="Send to back" disabled={layerIndex <= 1} onClick={() => onReorder("back")} aria-label="Send to back">
              <Icon name="chevronsDown" size={15} />
            </button>
            <button type="button" className="btn btn--ghost btn--sm" title="Send backward" disabled={layerIndex <= 1} onClick={() => onReorder("backward")} aria-label="Send backward">
              <Icon name="chevronDown" size={15} />
            </button>
            <button type="button" className="btn btn--ghost btn--sm" title="Bring forward" disabled={layerIndex >= layerCount} onClick={() => onReorder("forward")} aria-label="Bring forward">
              <Icon name="chevronUp" size={15} />
            </button>
            <button type="button" className="btn btn--ghost btn--sm" title="Bring to front" disabled={layerIndex >= layerCount} onClick={() => onReorder("front")} aria-label="Bring to front">
              <Icon name="chevronsUp" size={15} />
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

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
function DraftNumber({
  label,
  hideLabel,
  inline,
  value,
  min,
  max,
  suffix,
  onCommit,
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
}) {
  const [focused, setFocused] = useState(false);
  const [draft, setDraft] = useState(formatNum(value));
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
      className="pdf-editor__num-input"
      inputMode="decimal"
      value={draft}
      aria-label={label}
      onFocus={() => setFocused(true)}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={(e) => {
        setFocused(false);
        commit(e.target.value);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") (e.target as HTMLInputElement).blur();
        if (e.key === "Escape") {
          setDraft(formatNum(value));
          (e.target as HTMLInputElement).blur();
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

function ColorField({
  label,
  icon,
  value,
  active,
  onChange,
  onPickFromPage,
}: {
  label: string;
  icon?: "droplet" | "square" | "squareFill";
  value: string;
  active?: boolean;
  onChange: (hex: string) => void;
  onPickFromPage?: () => void;
}) {
  const hex = toCssHex(value, "#111827");
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
        {PRESETS.map((c) => (
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
