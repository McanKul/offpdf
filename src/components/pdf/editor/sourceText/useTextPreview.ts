/**
 * The real render of a page with its committed text changes (SPEC §D.4, §D.7).
 *
 * Whenever the page's set of changes differs (commit, restore, undo, redo,
 * page change) the page is previewed through `preview_text_edits` — the same
 * plan + qpdf write + checks as Save — unless that exact set is cached (LRU of
 * 8, keyed by `editsSignature`). A generation counter drops answers for a set
 * that is no longer current. No changes ⇒ no bytes (the original render).
 *
 * `request` runs a preview for a candidate set without touching the shown
 * page (the inline editor's commit); `seed` stores its answer so the page
 * shows it as soon as the commit lands, without a second call.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { editsSignature, toTextEditIn, type SourceTextObject } from "@/lib/editor";
import { base64ToBytes } from "@/lib/pdfjs";
import { previewTextEdits } from "@/lib/tauriCommands";
import { toAppError, type AppError, type TextEditIn, type TextPreview } from "@/lib/types";
import { lruGet, lruPut } from "./usePageText";

export const PREVIEW_CACHE_PAGES = 8;

export type TextPreviewStatus = "idle" | "pending" | "ready" | "unavailable" | "error";

export interface TextPreviewArgs {
  path: string | null;
  fingerprint: string | null;
  /** 0-based page in the source file. */
  sourcePageIndex: number | null;
  /** The page's committed text changes. */
  objects: SourceTextObject[];
}

export interface TextPreviewState {
  /** One-page PDF to render instead of the original, or null. */
  bytes: Uint8Array | null;
  status: TextPreviewStatus;
  /** The answer for the current set of changes. */
  result: TextPreview | null;
  error: AppError | null;
}

export interface TextPreviewController extends TextPreviewState {
  request(edits: TextEditIn[]): Promise<TextPreview>;
  seed(signature: string, preview: TextPreview): void;
}

interface Entry {
  preview: TextPreview;
  bytes: Uint8Array | null;
}

interface Snapshot {
  /** Page identity (`path`, fingerprint, page); bytes never cross pages. */
  base: string | null;
  owner: number;
  key: string | null;
  status: TextPreviewStatus;
  entry: Entry | null;
  /** Bytes shown while the current set is pending (same page only). */
  shown: Uint8Array | null;
  error: AppError | null;
}

const IDLE: Snapshot = { base: null, owner: -1, key: null, status: "idle", entry: null, shown: null, error: null };

function isPreview(value: unknown): value is TextPreview {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Partial<TextPreview>;
  return Array.isArray(v.verdicts) && Array.isArray(v.warnings) && (v.pagePdf === null || typeof v.pagePdf === "string");
}

function unexpectedAnswer(): AppError {
  return { ...toAppError(null), details: "preview_text_edits returned an unexpected answer" };
}

function toEntry(preview: TextPreview): Entry {
  return { preview, bytes: preview.pagePdf ? base64ToBytes(preview.pagePdf) : null };
}

const statusOf = (entry: Entry): TextPreviewStatus => (entry.bytes ? "ready" : "unavailable");

export function useTextPreview({ path, fingerprint, sourcePageIndex, objects }: TextPreviewArgs): TextPreviewController {
  const cache = useRef(new Map<string, Entry>());
  const generation = useRef(0);
  const objectsRef = useRef(objects);
  objectsRef.current = objects;
  const [snap, setSnap] = useState<Snapshot>(IDLE);

  const base = path && fingerprint && sourcePageIndex !== null ? `${path}\u0000${fingerprint}\u0000${sourcePageIndex}` : null;
  /** Editor page the changes belong to: a source page listed twice never shows the other copy's bytes. */
  const owner = objects[0]?.pageIndex ?? -1;
  const signature = editsSignature(objects);
  const key = base && objects.length > 0 ? `${base}\u0000${signature}` : null;
  const baseRef = useRef(base);
  baseRef.current = base;
  const keyRef = useRef(key);
  keyRef.current = key;
  const ownerRef = useRef(owner);
  ownerRef.current = owner;

  useEffect(() => {
    const gen = ++generation.current;
    if (!key || !base || !path || !fingerprint || sourcePageIndex === null) {
      setSnap(IDLE);
      return;
    }
    const hit = lruGet(cache.current, key);
    if (hit) {
      setSnap({ base, owner, key, status: statusOf(hit), entry: hit, shown: hit.bytes, error: null });
      return;
    }
    setSnap((prev) => ({
      base,
      owner,
      key,
      status: "pending",
      entry: null,
      shown: prev.base === base && prev.owner === owner ? prev.shown : null,
      error: null,
    }));
    previewTextEdits(path, fingerprint, sourcePageIndex, objectsRef.current.map(toTextEditIn)).then(
      (preview) => {
        if (gen !== generation.current) return;
        if (!isPreview(preview)) {
          setSnap({ base, owner, key, status: "error", entry: null, shown: null, error: unexpectedAnswer() });
          return;
        }
        const entry = toEntry(preview);
        lruPut(cache.current, key, entry, PREVIEW_CACHE_PAGES);
        setSnap({ base, owner, key, status: statusOf(entry), entry, shown: entry.bytes, error: null });
      },
      (e: unknown) => {
        if (gen !== generation.current) return;
        setSnap({ base, owner, key, status: "error", entry: null, shown: null, error: toAppError(e) });
      },
    );
  }, [key, base, owner, path, fingerprint, sourcePageIndex]);

  const request = useCallback(
    (edits: TextEditIn[]): Promise<TextPreview> => {
      if (!path || !fingerprint || sourcePageIndex === null) {
        return Promise.reject({ ...toAppError(null), details: "Edit text source is not open" });
      }
      return previewTextEdits(path, fingerprint, sourcePageIndex, edits).then((preview) => {
        if (!isPreview(preview)) throw unexpectedAnswer();
        return preview;
      });
    },
    [path, fingerprint, sourcePageIndex],
  );

  const seed = useCallback((sig: string, preview: TextPreview) => {
    const pageBase = baseRef.current;
    if (!pageBase || !isPreview(preview)) return;
    const seededKey = `${pageBase}\u0000${sig}`;
    const entry = toEntry(preview);
    lruPut(cache.current, seededKey, entry, PREVIEW_CACHE_PAGES);
    if (seededKey === keyRef.current) {
      generation.current += 1;
      setSnap({ base: pageBase, owner: ownerRef.current, key: seededKey, status: statusOf(entry), entry, shown: entry.bytes, error: null });
    }
  }, []);

  if (!key || snap.key !== key) {
    return { bytes: key && snap.base === base && snap.owner === owner ? snap.shown : null, status: key ? "pending" : "idle", result: null, error: null, request, seed };
  }
  return { bytes: snap.shown, status: snap.status, result: snap.entry?.preview ?? null, error: snap.error, request, seed };
}
