import { useEffect, useRef, useState, type ReactNode } from "react";
import { Tabs } from "@/components/ui/Tabs";
import { PageRangeInput } from "./PageRangeInput";
import { PageThumbnails } from "./PageThumbnails";
import { PageOrderGrid } from "./PageOrderGrid";
import { PageLightbox } from "./PageLightbox";
import { useThumbnails } from "./useThumbnails";
import { parsePageRange, formatPageList } from "@/lib/pageRange";
import { rendererAvailable } from "@/lib/tauriCommands";
import type { FileInfo } from "@/lib/types";

/**
 * Page selector with two interchangeable tabs:
 *  - "Visual" — render page thumbnails and click/drag to choose (default)
 *  - "Type"   — the text page-range input
 *
 * Keeps the same string `value`/`onChange` contract as PageRangeInput so tool
 * pages drop it in with one extra `file` prop. The thumbnail cache lives here,
 * so switching tabs never re-renders pages. The Visual tab only appears when a
 * local renderer is available and the file's page count is known.
 */
export function PagePicker({
  file,
  value,
  onChange,
  pageCount,
  mode = "set",
  allowAll = true,
  label,
  hint,
  onValidChange,
}: {
  file: FileInfo | undefined;
  value: string;
  onChange: (value: string) => void;
  pageCount?: number;
  mode?: "set" | "order";
  allowAll?: boolean;
  label?: ReactNode;
  hint?: ReactNode;
  onValidChange?: (valid: boolean) => void;
}) {
  const [canRender, setCanRender] = useState(false);
  const [tab, setTab] = useState<"type" | "visual">("visual");
  const [zoomPage, setZoomPage] = useState<number | null>(null);
  const userPicked = useRef(false);

  // Shared thumbnail cache (persists while the file is loaded, across tabs).
  const thumbs = useThumbnails(file);

  useEffect(() => {
    let active = true;
    rendererAvailable()
      .then((v) => active && setCanRender(v))
      .catch(() => active && setCanRender(false));
    return () => {
      active = false;
    };
  }, []);

  const canVisual = canRender && !!file && (file.pageCount ?? 0) > 0;

  // New file → forget any manual tab choice so preview is the default again.
  useEffect(() => {
    userPicked.current = false;
  }, [file?.path]);

  // Default to the visual tab whenever possible, unless the user picked a tab.
  useEffect(() => {
    if (canVisual && !userPicked.current) setTab("visual");
    else if (!canVisual) setTab("type");
  }, [canVisual, file?.path]);

  const pickTab = (t: "type" | "visual") => {
    userPicked.current = true;
    setTab(t);
  };

  // Report validity to the parent (single source, regardless of active tab).
  useEffect(() => {
    const ok =
      value.trim() !== "" &&
      parsePageRange(value, { pageCount, preserveOrder: mode === "order", allowAll }).ok;
    onValidChange?.(ok);
  }, [value, pageCount, mode, allowAll, onValidChange]);

  // In order mode, seed the identity order once per active file (so the grid is
  // populated and the field is immediately valid). Keyed on the file path so it
  // re-seeds when the active document changes, but never clobbers user edits of
  // the same file. Not gated on `value` so a parent reset can't deadlock it.
  const seededPath = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (mode !== "order" || !file || (file.pageCount ?? 0) === 0) return;
    if (seededPath.current === file.path) return;
    seededPath.current = file.path;
    onChange(formatPageList(Array.from({ length: file.pageCount as number }, (_, i) => i + 1)));
  }, [mode, file, onChange]);

  const parsed = parsePageRange(value, {
    pageCount,
    preserveOrder: mode === "order",
    allowAll,
  });
  const currentPages = parsed.ok ? parsed.pages : [];
  const showVisual = tab === "visual" && canVisual;

  return (
    <div className="col">
      {(label || canVisual) && (
        <div className="spread" style={{ gap: 12 }}>
          {label ? <span className="field__label">{label}</span> : <span />}
          {canVisual && (
            <Tabs
              tabs={[
                { id: "visual", label: mode === "order" ? "Reorder visually" : "Pick visually" },
                { id: "type", label: mode === "order" ? "Type order" : "Type pages" },
              ]}
              active={tab}
              onChange={(t) => pickTab(t)}
            />
          )}
        </div>
      )}

      {!showVisual ? (
        <PageRangeInput
          value={value}
          onChange={onChange}
          pageCount={pageCount}
          mode={mode}
          allowAll={allowAll}
          hint={hint}
        />
      ) : mode === "order" ? (
        <PageOrderGrid
          file={file as FileInfo}
          value={currentPages}
          onChange={(o) => onChange(formatPageList(o))}
          onZoom={setZoomPage}
          thumbs={thumbs}
        />
      ) : (
        <PageThumbnails
          file={file as FileInfo}
          value={currentPages}
          onChange={(pages) => onChange(formatPageList(pages))}
          onZoom={setZoomPage}
          thumbs={thumbs}
        />
      )}

      <PageLightbox
        file={file}
        page={zoomPage}
        pageCount={file?.pageCount ?? pageCount ?? 0}
        onChange={setZoomPage}
        onClose={() => setZoomPage(null)}
      />
    </div>
  );
}
