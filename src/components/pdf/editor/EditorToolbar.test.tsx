import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { UI } from "@/lib/editor/sourceTextCopy";
import type { EditorTool } from "./EditorOverlay";
import { DEFAULT_EDITOR_TOOL, EditorToolbar, editorShortcut, type EditorToolbarProps } from "./EditorToolbar";

const noop = () => {};

function render(tool: EditorTool, showOriginal: Partial<EditorToolbarProps["showOriginal"]> = {}): string {
  return renderToStaticMarkup(
    <EditorToolbar
      tool={tool}
      onTool={noop}
      onImage={noop}
      shapeOpen={false}
      onShapeOpenChange={noop}
      lastShape="rect"
      onPickShape={noop}
      history={{
        canUndo: false,
        canRedo: false,
        onUndo: noop,
        onRedo: noop,
        canCopy: false,
        onCopy: noop,
        canPaste: false,
        onPaste: noop,
      }}
      showOriginal={{ visible: false, pressed: false, onToggle: noop, ...showOriginal }}
      zoom={1}
      onZoom={noop}
      pageIndex={0}
      pageCount={3}
      onPage={noop}
    />,
  );
}

function buttons(markup: string): string[] {
  return [...markup.matchAll(/<button\b[^>]*>/g)].map((m) => m[0]);
}

function attr(tag: string, name: string): string | undefined {
  return tag.match(new RegExp(`\\s${name}="([^"]*)"`))?.[1];
}

function byLabel(markup: string, label: string): string | undefined {
  return buttons(markup).find((b) => attr(b, "aria-label") === label);
}

const KEY = { metaKey: false, ctrlKey: false, altKey: false };

describe("EditorToolbar", () => {
  it("lists the tools in the spec order: Select · Hand · Edit text · Add text · Image · Draw · Link | Shapes | Redaction | Markup", () => {
    const labels = buttons(render("select"))
      .filter((b) => attr(b, "aria-pressed") !== undefined || attr(b, "aria-label") === "Shapes")
      .map((b) => attr(b, "aria-label"));
    expect(labels).toEqual([
      "Select",
      "Hand",
      "Edit text",
      "Add text",
      "Image",
      "Draw",
      "Link",
      "Shapes",
      "Redaction",
      "Note",
      "Highlight",
      "Underline",
      "Strikeout",
      "Ink annot",
    ]);
  });

  it("opens on Select (D34): the default tool is Select and it is the pressed button", () => {
    expect(DEFAULT_EDITOR_TOOL).toBe("select");
    const markup = render(DEFAULT_EDITOR_TOOL);
    expect(attr(byLabel(markup, "Select")!, "aria-pressed")).toBe("true");
    expect(attr(byLabel(markup, "Add text")!, "aria-pressed")).toBe("false");
    expect(attr(byLabel(markup, "Edit text")!, "aria-pressed")).toBe("false");
  });

  it("titles and labels the two text tools from the copy deck", () => {
    const markup = render("select");
    expect(attr(byLabel(markup, "Edit text")!, "title")).toBe(UI.tool.editText.title);
    expect(attr(byLabel(markup, "Add text")!, "title")).toBe(UI.tool.addText.title);
    expect(markup).not.toContain('aria-label="Text"');
  });

  it("shows the Add text banner only while Add text is active", () => {
    expect(render("text")).toContain(UI.banner.addText);
    expect(render("select")).not.toContain(UI.banner.addText);
    expect(render("editText")).not.toContain(UI.banner.addText);
  });

  it("announces E and O with aria-keyshortcuts (H for Hand)", () => {
    const markup = render("editText", { visible: true });
    expect(attr(byLabel(markup, "Edit text")!, "aria-keyshortcuts")).toBe("E");
    expect(attr(byLabel(markup, "Hand")!, "aria-keyshortcuts")).toBe("H");
    const show = buttons(markup).find((b) => attr(b, "title") === UI.tool.showOriginal.title);
    expect(show).toBeDefined();
    expect(attr(show!, "aria-keyshortcuts")).toBe("O");
  });

  it("offers Show original only when the page has text changes, as a toggle", () => {
    expect(render("select")).not.toContain(UI.tool.showOriginal.label);
    const off = render("select", { visible: true, pressed: false });
    const on = render("select", { visible: true, pressed: true });
    const find = (m: string) => buttons(m).find((b) => attr(b, "title") === UI.tool.showOriginal.title)!;
    expect(attr(find(off), "aria-pressed")).toBe("false");
    expect(attr(find(on), "aria-pressed")).toBe("true");
    expect(on).toContain(`>${UI.tool.showOriginal.label}</button>`);
  });

  it("maps E to Edit text, H to Hand and O to Show original (only when offered)", () => {
    expect(editorShortcut({ ...KEY, key: "e" }, false)).toEqual({ kind: "tool", tool: "editText" });
    expect(editorShortcut({ ...KEY, key: "E" }, false)).toEqual({ kind: "tool", tool: "editText" });
    expect(editorShortcut({ ...KEY, key: "h" }, false)).toEqual({ kind: "tool", tool: "hand" });
    expect(editorShortcut({ ...KEY, key: "o" }, true)).toEqual({ kind: "showOriginal" });
    expect(editorShortcut({ ...KEY, key: "o" }, false)).toBeNull();
  });

  it("leaves modified keys alone (⌘E, Ctrl+O, Alt+E)", () => {
    expect(editorShortcut({ ...KEY, key: "e", metaKey: true }, true)).toBeNull();
    expect(editorShortcut({ ...KEY, key: "o", ctrlKey: true }, true)).toBeNull();
    expect(editorShortcut({ ...KEY, key: "e", altKey: true }, true)).toBeNull();
    expect(editorShortcut({ ...KEY, key: "x" }, true)).toBeNull();
  });
});
