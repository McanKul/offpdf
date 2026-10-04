/**
 * Editor toolbar (extracted from `PdfEditorCanvas`): history and clipboard,
 * the tools in the order Select · Hand · Edit text · Add text · Image · Draw ·
 * Link | Shapes | Redaction | Markup | Show original | zoom | pages, and the
 * Add text banner. Keyboard shortcuts are resolved by `editorShortcut`.
 */
import { Alert } from "@/components/ui/Alert";
import { Button } from "@/components/ui/Button";
import { Icon, type IconName } from "@/components/ui/Icon";
import { UI } from "@/lib/editor/sourceTextCopy";
import type { EditorTool } from "./EditorOverlay";
import { ShapePicker, SHAPE_TOOLS } from "./ShapePicker";

/** Edit PDF opens on Select, so a first click on existing words never drops a new box (D34). */
export const DEFAULT_EDITOR_TOOL: EditorTool = "select";

interface ToolButton {
  id: EditorTool;
  label: string;
  title: string;
  icon: IconName;
  shortcut?: string;
}

export const MAIN_TOOLS: readonly ToolButton[] = [
  { id: "select", label: "Select", title: "Select", icon: "mousePointer" },
  { id: "hand", label: "Hand", title: "Hand — drag to slide the page (H, hold Space)", icon: "hand", shortcut: "H" },
  { id: "editText", label: UI.tool.editText.label, title: UI.tool.editText.title, icon: "textCursor", shortcut: "E" },
  { id: "text", label: UI.tool.addText.label, title: UI.tool.addText.title, icon: "type" },
  { id: "image", label: "Image", title: "Image", icon: "image" },
  { id: "ink", label: "Draw", title: "Draw", icon: "pencil" },
  { id: "link", label: "Link", title: "Link — draw a hotspot (does not open the address)", icon: "external" },
];

const MARKUP_TOOLS: readonly ToolButton[] = [
  { id: "note", label: "Note", title: "Note", icon: "badge" },
  { id: "highlight", label: "Highlight", title: "Highlight", icon: "sparkles" },
  { id: "underline", label: "Underline", title: "Underline", icon: "type" },
  { id: "strikeout", label: "Strikeout", title: "Strikeout", icon: "slash" },
  { id: "markupInk", label: "Ink annot", title: "Ink annot", icon: "stamp" },
];

export type EditorShortcut = { kind: "tool"; tool: EditorTool } | { kind: "showOriginal" };

/** Single-key editor shortcuts (not typing): H hand, E Edit text, O Show original (only when offered). */
export function editorShortcut(
  e: { key: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean },
  canShowOriginal: boolean,
): EditorShortcut | null {
  if (e.metaKey || e.ctrlKey || e.altKey) return null;
  const key = e.key.toLowerCase();
  if (key === "h") return { kind: "tool", tool: "hand" };
  if (key === "e") return { kind: "tool", tool: "editText" };
  if (key === "o" && canShowOriginal) return { kind: "showOriginal" };
  return null;
}

type ShapeId = (typeof SHAPE_TOOLS)[number]["id"];

export interface EditorToolbarProps {
  tool: EditorTool;
  onTool: (tool: EditorTool) => void;
  onImage: () => void;
  shapeOpen: boolean;
  onShapeOpenChange: (open: boolean) => void;
  lastShape: ShapeId;
  onPickShape: (id: ShapeId) => void;
  history: {
    canUndo: boolean;
    canRedo: boolean;
    onUndo: () => void;
    onRedo: () => void;
    canCopy: boolean;
    onCopy: () => void;
    canPaste: boolean;
    onPaste: () => void;
  };
  /** Shown only when the current page has text changes. */
  showOriginal: { visible: boolean; pressed: boolean; onToggle: () => void };
  zoom: number;
  /** −1 zoom out, 0 reset, +1 zoom in. */
  onZoom: (step: -1 | 0 | 1) => void;
  pageIndex: number;
  pageCount: number;
  onPage: (delta: number) => void;
}

function ToolToggle({ t, tool, onClick }: { t: ToolButton; tool: EditorTool; onClick: () => void }) {
  return (
    <Button
      size="sm"
      variant={tool === t.id ? "primary" : "ghost"}
      title={t.title}
      aria-label={t.label}
      aria-pressed={tool === t.id}
      aria-keyshortcuts={t.shortcut}
      onClick={onClick}
    >
      <Icon name={t.icon} size={16} />
    </Button>
  );
}

export function EditorToolbar(props: EditorToolbarProps) {
  const { tool, onTool, history, showOriginal, pageIndex, pageCount } = props;
  return (
    <>
      <div className="pdf-editor__toolbar thumb-toolbar wrap">
        <Button size="sm" variant="secondary" onClick={history.onUndo} disabled={!history.canUndo} title="Undo" aria-label="Undo">
          <Icon name="undo" size={15} />
        </Button>
        <Button size="sm" variant="secondary" onClick={history.onRedo} disabled={!history.canRedo} title="Redo" aria-label="Redo">
          <Icon name="undo" size={15} style={{ transform: "scaleX(-1)" }} />
        </Button>
        <Button size="sm" variant="ghost" onClick={history.onCopy} disabled={!history.canCopy} title="Copy (Ctrl/Cmd+C)" aria-label="Copy">
          <Icon name="copy" size={15} />
        </Button>
        <Button size="sm" variant="ghost" onClick={history.onPaste} disabled={!history.canPaste} title="Paste (Ctrl/Cmd+V)" aria-label="Paste">
          <Icon name="clipboard" size={15} />
        </Button>
        <span className="pdf-editor__sep" />
        {MAIN_TOOLS.map((t) => (
          <ToolToggle key={t.id} t={t} tool={tool} onClick={() => (t.id === "image" ? props.onImage() : onTool(t.id))} />
        ))}
        <span className="pdf-editor__sep" />
        <ShapePicker
          tool={tool}
          open={props.shapeOpen}
          lastShape={props.lastShape}
          onOpenChange={props.onShapeOpenChange}
          onPick={props.onPickShape}
        />
        <span className="pdf-editor__sep" />
        <Button
          size="sm"
          variant={tool === "redact" ? "primary" : "ghost"}
          title="Redaction — permanently remove content in this region on Save"
          aria-label="Redaction"
          aria-pressed={tool === "redact"}
          onClick={() => onTool("redact")}
        >
          <Icon name="squareFill" size={16} />
        </Button>
        <span className="pdf-editor__sep" />
        {MARKUP_TOOLS.map((t) => (
          <ToolToggle key={t.id} t={t} tool={tool} onClick={() => onTool(t.id)} />
        ))}
        {showOriginal.visible && (
          <>
            <span className="pdf-editor__sep" />
            <Button
              size="sm"
              variant={showOriginal.pressed ? "primary" : "ghost"}
              title={UI.tool.showOriginal.title}
              aria-pressed={showOriginal.pressed}
              aria-keyshortcuts="O"
              onClick={showOriginal.onToggle}
            >
              {UI.tool.showOriginal.label}
            </Button>
          </>
        )}
        <span className="pdf-editor__sep" />
        <span className="pdf-editor__toolgroup">
          <Button size="sm" variant="ghost" onClick={() => props.onZoom(-1)} title="Zoom out" aria-label="Zoom out">
            <Icon name="minus" size={15} />
          </Button>
          <button type="button" className="btn btn--ghost btn--sm" title="Reset zoom" aria-label="Reset zoom" onClick={() => props.onZoom(0)} style={{ minWidth: 44 }}>
            {Math.round(props.zoom * 100)}%
          </button>
          <Button size="sm" variant="ghost" onClick={() => props.onZoom(1)} title="Zoom in" aria-label="Zoom in">
            <Icon name="plus" size={15} />
          </Button>
        </span>
        <span className="pdf-editor__sep" />
        <span className="pdf-editor__toolgroup">
          <Button size="sm" variant="ghost" onClick={() => props.onPage(-1)} disabled={pageIndex <= 0} title="Previous page" aria-label="Previous page">
            <Icon name="chevronRight" size={15} style={{ transform: "rotate(180deg)" }} />
          </Button>
          <span className="muted" style={{ fontSize: 12.5 }}>
            {pageIndex + 1} / {pageCount}
          </span>
          <Button size="sm" variant="ghost" onClick={() => props.onPage(1)} disabled={pageIndex >= pageCount - 1} title="Next page" aria-label="Next page">
            <Icon name="chevronRight" size={15} />
          </Button>
        </span>
      </div>
      {tool === "text" && <Alert variant="info">{UI.banner.addText}</Alert>}
    </>
  );
}
