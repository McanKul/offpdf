import type { EditObject, LayerDir, SourceTextObject } from "@/lib/editor";
import {
  TEXT_INKS,
  isClosedShapeObject,
  isMarkupObject,
  isNoneFill,
  sizeWithAspect,
  surfaceFor,
  toCssHex,
} from "@/lib/editor";
import { UI, fontDescription, truncateCopy } from "@/lib/editor/sourceTextCopy";
import type { TextFont, TextRun } from "@/lib/types";
import { Icon } from "@/components/ui/Icon";
import { ColorField as SharedColorField, DraftNumber } from "./controls";

const PRESETS = [
  "#111827",
  "#ffffff",
  "#dc2626",
  "#16a34a",
  "#2563eb",
  "#6b7280",
];

export type ColorPickTarget = "color" | "fill" | "stroke";

/** The object inspector's colour field: its presets, the first one when unreadable. */
function ColorField(props: Omit<Parameters<typeof SharedColorField>[0], "presets" | "fallback">) {
  return <SharedColorField {...props} presets={PRESETS} fallback={PRESETS[0]} />;
}

/** What the inspector needs to describe a text change (the run comes from the page's text). */
export interface SourceTextInspectorProps {
  run: TextRun | null;
  fonts: Map<string, TextFont>;
  onEditLine: () => void;
  onRestore: () => void;
}

export function ObjectInspector({
  obj,
  picking,
  layerIndex,
  layerCount,
  pageCount,
  onChange,
  onPickFromPage,
  onReorder,
  sourceText,
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
  /** Required to show a text change (`kind: "sourceText"`). */
  sourceText?: SourceTextInspectorProps;
}) {
  if (obj.kind === "sourceText") {
    return sourceText ? <SourceTextInspector obj={obj} {...sourceText} /> : null;
  }
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

      {obj.kind !== "link" && obj.kind !== "redact" && !markup && <label className="field__label">Opacity</label>}
      {obj.kind !== "link" && obj.kind !== "redact" && !markup && (
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

const LETTERS_MAX_CHARS = 120;

function formatPt(n: number): string {
  return `${Number(n.toFixed(2))} pt`;
}

/** The letters a face can type: space named, the rest in font order, cut at 120. */
function lettersOf(run: TextRun, fonts: Map<string, TextFont>, face: SourceTextObject["style"]["face"]): string {
  const seen = new Set<string>();
  for (const key of surfaceFor(run, face)) {
    for (const ch of Array.from(fonts.get(key)?.alphabet ?? "")) seen.add(ch);
  }
  const letters = [...seen].filter((ch) => ch !== " ").join("");
  return truncateCopy(seen.has(" ") ? `${UI.spaceName} ${letters}` : letters, LETTERS_MAX_CHARS);
}

function styleSummary(obj: SourceTextObject, run: TextRun | null): string {
  const parts: string[] = [];
  const size = obj.style.sizePt ?? run?.metrics?.effectiveSize;
  if (size !== undefined) parts.push(formatPt(size));
  const face = obj.style.face ?? run?.style?.face;
  if (face) parts.push(UI.faceLabel[face]);
  const fill = (obj.style.fill ?? run?.style?.fill ?? "").toLowerCase();
  const ink = TEXT_INKS.find((i) => i.hex === fill);
  if (ink) parts.push(UI.bar[ink.key]);
  else if (fill) parts.push(fill);
  const spacing = obj.style.letterSpacingPt;
  if (spacing !== undefined) parts.push(`${UI.bar.letterSpacing} ${formatPt(spacing)}`);
  return parts.join(" · ");
}

/** Inspector branch for a text change (§D.6.4): read-only rows and two actions. */
function SourceTextInspector({
  obj,
  run,
  fonts,
  onEditLine,
  onRestore,
}: { obj: SourceTextObject } & SourceTextInspectorProps) {
  const primary = run ? fonts.get(surfaceFor(run, obj.style.face)[0] ?? "") : undefined;
  const subset = !!primary && primary.embedded && primary.subset;
  const summary = styleSummary(obj, run);
  return (
    <div className="pdf-editor__inspector st-inspector">
      <div className="pdf-editor__sidebar-title">{UI.inspector.header}</div>
      <dl className="st-inspector__rows">
        <dt>{UI.inspector.original}</dt>
        <dd>{obj.originalText}</dd>
        <dt>{UI.inspector.new}</dt>
        <dd>{obj.text === "" ? UI.removal : obj.text}</dd>
        {primary && (
          <>
            <dt>{UI.inspector.font}</dt>
            <dd>{fontDescription(primary)}</dd>
          </>
        )}
        {subset && run && (
          <>
            <dt>{UI.bar.letters}</dt>
            <dd className="st-inspector__letters">{lettersOf(run, fonts, obj.style.face)}</dd>
          </>
        )}
        {summary && (
          <>
            <dt>{UI.inspector.style}</dt>
            <dd>{summary}</dd>
          </>
        )}
      </dl>
      <p className="st-inspector__note">{UI.inspector.note}</p>
      <div className="st-inspector__actions">
        <button type="button" className="btn btn--secondary btn--sm" onClick={onEditLine}>
          <Icon name="textCursor" size={15} />
          {UI.inspector.editLine}
        </button>
        <button type="button" className="btn btn--ghost btn--sm" onClick={onRestore}>
          <Icon name="undo" size={15} />
          {UI.inspector.restore}
        </button>
      </div>
    </div>
  );
}
