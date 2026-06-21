/** In-app full-text search over the combined document. Text is extracted
 * natively (pdftotext) — only text strings cross IPC, never the PDF bytes — so
 * it's light on memory even for big files. Clicking a hit opens that page.
 *
 * For scanned/image PDFs (no text layer), the user can OCR on the fly: it runs
 * OCR to a temp file purely so the preview becomes searchable WITHOUT saving;
 * they can then optionally save that searchable copy. The OCR'd text is keyed by
 * page ref key, so it survives reordering and page changes. */
import { useEffect, useRef, useState } from "react";
import { Icon } from "@/components/ui/Icon";
import { Button } from "@/components/ui/Button";
import { Spinner } from "@/components/ui/Spinner";
import { useToast } from "@/components/ui/Toast";
import {
  pdfText,
  ocrPdf,
  getTempDir,
  copyFile,
  pickOutputFolder,
  onJobUpdate,
  newJobId,
} from "@/lib/tauriCommands";
import { joinPath } from "@/lib/validation";
import { toAppError, type PageRef } from "@/lib/types";
import { buildPicks } from "./useCombinedDoc";

const cache = new Map<string, string[]>();

interface Hit {
  ref: PageRef;
  before: string;
  match: string;
  after: string;
}

function snippet(
  text: string,
  q: string,
  caseSensitive: boolean,
): { before: string; match: string; after: string } | null {
  const idx = caseSensitive ? text.indexOf(q) : text.toLowerCase().indexOf(q.toLowerCase());
  if (idx < 0) return null;
  const start = Math.max(0, idx - 40);
  const before = (start > 0 ? "…" : "") + text.slice(start, idx).replace(/\s+/g, " ");
  const match = text.slice(idx, idx + q.length);
  const after = (text.slice(idx + q.length, idx + q.length + 60) + "…").replace(/\s+/g, " ");
  return { before, match, after };
}

export function PdfSearch({ refs, onOpen }: { refs: PageRef[]; onOpen: (ref: PageRef) => void }) {
  const { toast } = useToast();
  const [query, setQuery] = useState("");
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [, force] = useState(0);
  const [loading, setLoading] = useState(false);
  const loadedPaths = useRef<Set<string>>(new Set());

  // On-the-fly OCR results, keyed by page ref key (survives reorder).
  const [ocrMap, setOcrMap] = useState<Record<string, string>>({});
  const [ocrBusy, setOcrBusy] = useState(false);
  const [ocrPct, setOcrPct] = useState(0);
  const [ocrOut, setOcrOut] = useState<string | null>(null);

  const q = query.trim();

  const textFor = (ref: PageRef): string => cache.get(ref.path)?.[ref.page - 1] || ocrMap[ref.key] || "";

  // Lazily extract native text for the loaded files the first time the user searches.
  useEffect(() => {
    if (q.length < 2) return;
    const paths = Array.from(new Set(refs.map((r) => r.path))).filter((p) => !loadedPaths.current.has(p));
    if (paths.length === 0) return;
    let active = true;
    setLoading(true);
    (async () => {
      for (const p of paths) {
        try {
          cache.set(p, await pdfText(p));
        } catch {
          cache.set(p, []);
        }
        loadedPaths.current.add(p);
      }
      if (active) {
        setLoading(false);
        force((n) => n + 1);
      }
    })();
    return () => {
      active = false;
    };
  }, [q, refs]);

  const hits: Hit[] = [];
  if (q.length >= 2) {
    for (const ref of refs) {
      const s = snippet(textFor(ref), q, caseSensitive);
      if (s) hits.push({ ref, ...s });
      if (hits.length >= 100) break;
    }
  }

  const allLoaded = Array.from(new Set(refs.map((r) => r.path))).every((p) => loadedPaths.current.has(p));
  const hasAnyText = refs.some((r) => textFor(r).trim().length > 0);
  const noTextLayer = q.length >= 2 && !loading && allLoaded && !hasAnyText;

  const runOcr = async () => {
    setOcrBusy(true);
    setOcrPct(0);
    const jobId = newJobId();
    let un: undefined | (() => void);
    try {
      const dir = await getTempDir();
      const out = joinPath(dir, `search-${jobId}.pdf`);
      un = await onJobUpdate((u) => setOcrPct(Math.round(u.percent ?? 0)), jobId);
      await ocrPdf(jobId, out, buildPicks(refs), "eng+tur");
      const texts = await pdfText(out);
      const map: Record<string, string> = {};
      refs.forEach((r, i) => {
        map[r.key] = texts[i] ?? "";
      });
      setOcrMap((prev) => ({ ...prev, ...map }));
      setOcrOut(out);
    } catch (e) {
      toast({ title: "OCR failed", description: toAppError(e).message, variant: "error" });
    } finally {
      un?.();
      setOcrBusy(false);
    }
  };

  const saveSearchable = async () => {
    if (!ocrOut) return;
    try {
      const folder = await pickOutputFolder();
      if (!folder) return;
      const dst = await copyFile(ocrOut, joinPath(folder, "searchable.pdf"));
      toast({ title: "Saved searchable PDF", description: dst, variant: "success" });
    } catch (e) {
      toast({ title: "Couldn't save", description: toAppError(e).message, variant: "error" });
    }
  };

  return (
    <div className="col gap-sm">
      <div className="field">
        <div className="input" style={{ display: "flex", alignItems: "center", gap: 6, padding: 0, paddingLeft: 12 }}>
          <Icon name="search" size={15} className="subtle" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search text in the document…"
            spellCheck={false}
            style={{ border: "none", outline: "none", background: "transparent", color: "inherit", flex: 1, padding: "9px 8px 9px 0", font: "inherit" }}
          />
          {loading && <Spinner className="btn__spinner" />}
          <button
            className="btn btn--ghost btn--sm"
            onClick={() => setCaseSensitive((v) => !v)}
            title={caseSensitive ? "Match case: on" : "Match case: off"}
            aria-pressed={caseSensitive}
            style={{
              fontWeight: 700,
              fontSize: 12.5,
              minWidth: 30,
              color: caseSensitive ? "var(--primary)" : "var(--text-subtle)",
              background: caseSensitive ? "var(--primary-soft)" : "transparent",
            }}
          >
            Aa
          </button>
          {query && (
            <button className="btn btn--ghost btn--sm" onClick={() => setQuery("")} title="Clear">
              <Icon name="x" size={15} />
            </button>
          )}
        </div>
      </div>

      {q.length >= 2 && !loading && !noTextLayer && (
        <div className="muted" style={{ fontSize: 12.5 }}>
          {hits.length === 0 ? "No matches." : `${hits.length}${hits.length >= 100 ? "+" : ""} match${hits.length === 1 ? "" : "es"}`}
          {ocrOut && (
            <>
              {" · "}
              <button className="linklike" onClick={saveSearchable}>
                Save searchable PDF…
              </button>
            </>
          )}
        </div>
      )}

      {noTextLayer && !ocrBusy && (
        <div className="row gap-sm" style={{ alignItems: "center" }}>
          <span className="muted" style={{ fontSize: 12.5 }}>
            No text layer (looks scanned).
          </span>
          <Button size="sm" variant="secondary" onClick={runOcr} leftIcon={<Icon name="scanText" size={14} />}>
            Make searchable (OCR)
          </Button>
        </div>
      )}

      {ocrBusy && (
        <div className="row gap-sm muted" style={{ fontSize: 12.5, alignItems: "center" }}>
          <Spinner /> Reading text with OCR… {ocrPct}% (no file is saved)
        </div>
      )}

      {hits.length > 0 && (
        <div className="search-results">
          {hits.map((h, i) => (
            <button key={`${h.ref.key}-${i}`} className="search-hit" onClick={() => onOpen(h.ref)}>
              <span className="search-hit__page">{h.ref.fileName} · p{h.ref.page}</span>
              <span className="search-hit__snippet">
                {h.before}
                <mark>{h.match}</mark>
                {h.after}
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
