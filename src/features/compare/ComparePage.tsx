import { useEffect, useState } from "react";
import { ToolPage, ToolSection } from "@/components/tools/ToolPage";
import { Button } from "@/components/ui/Button";
import { Icon } from "@/components/ui/Icon";
import { Spinner } from "@/components/ui/Spinner";
import { Alert } from "@/components/ui/Alert";
import { useToast } from "@/components/ui/Toast";
import { pickPdfFile, getFileInfo, renderThumbnails, diffPages } from "@/lib/tauriCommands";
import { getTool } from "@/lib/tools";
import { toAppError, type FileInfo, type DiffResult } from "@/lib/types";

const tool = getTool("compare");
const SIZE = 1100;

type Mode = "side" | "diff";

function FilePick({ label, info, onPick }: { label: string; info: FileInfo | null; onPick: () => void }) {
  return (
    <button className="cmp-pick" onClick={onPick}>
      <div className="cmp-pick__badge">{label}</div>
      <Icon name="fileText" size={18} className="muted" />
      <div className="grow truncate">
        {info ? (
          <>
            <div className="truncate" style={{ fontWeight: 600 }}>{info.name}</div>
            <div className="muted" style={{ fontSize: 12 }}>{info.pageCount ?? "?"} pages</div>
          </>
        ) : (
          <span className="muted">Choose PDF {label}…</span>
        )}
      </div>
      <Icon name="chevronRight" size={15} className="subtle" />
    </button>
  );
}

export function ComparePage() {
  const { toast } = useToast();
  const [a, setA] = useState<FileInfo | null>(null);
  const [b, setB] = useState<FileInfo | null>(null);
  const [page, setPage] = useState(1);
  const [mode, setMode] = useState<Mode>("side");

  const [aUrl, setAUrl] = useState<string | null>(null);
  const [bUrl, setBUrl] = useState<string | null>(null);
  const [diff, setDiff] = useState<DiffResult | null>(null);
  const [loading, setLoading] = useState(false);

  const aCount = a?.pageCount ?? 0;
  const bCount = b?.pageCount ?? 0;
  const maxPages = Math.max(aCount, bCount);
  const ready = !!a && !!b;

  const pick = async (which: "a" | "b") => {
    try {
      const path = await pickPdfFile();
      if (!path) return;
      const info = await getFileInfo(path);
      if (!info.isValidPdf) return toast({ title: "That file isn't a valid PDF", variant: "error" });
      which === "a" ? setA(info) : setB(info);
      setPage(1);
    } catch (e) {
      toast({ title: "Couldn't open the file", description: toAppError(e).message, variant: "error" });
    }
  };

  // Render the current page(s) / diff whenever inputs change.
  useEffect(() => {
    if (!ready) return;
    let active = true;
    setLoading(true);
    setDiff(null);
    (async () => {
      try {
        if (mode === "side") {
          const [ar, br] = await Promise.all([
            page <= aCount ? renderThumbnails(a!.path, [page], SIZE) : Promise.resolve([]),
            page <= bCount ? renderThumbnails(b!.path, [page], SIZE) : Promise.resolve([]),
          ]);
          if (!active) return;
          setAUrl(ar[0]?.dataUrl ?? null);
          setBUrl(br[0]?.dataUrl ?? null);
        } else {
          if (page <= aCount && page <= bCount) {
            const d = await diffPages(a!.path, page, b!.path, page, SIZE);
            if (active) setDiff(d);
          }
        }
      } catch (e) {
        if (active) toast({ title: "Compare failed", description: toAppError(e).message, variant: "error" });
      } finally {
        if (active) setLoading(false);
      }
    })();
    return () => {
      active = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [a?.path, b?.path, page, mode]);

  return (
    <ToolPage tool={tool}>
      <ToolSection label="Two PDFs" sublabel="Pick the two files to compare. They stay on your computer.">
        <div className="col">
          <FilePick label="A" info={a} onPick={() => pick("a")} />
          <FilePick label="B" info={b} onPick={() => pick("b")} />
        </div>
        {aCount > 0 && bCount > 0 && aCount !== bCount && (
          <div className="mt">
            <Alert variant="info">The files have a different number of pages ({aCount} vs {bCount}). Pages past the shorter file show blank.</Alert>
          </div>
        )}
      </ToolSection>

      {ready && (
        <ToolSection label="Compare">
          <div className="row spread wrap" style={{ gap: 10 }}>
            <div className="seg">
              <button className={`seg__btn ${mode === "side" ? "is-active" : ""}`} onClick={() => setMode("side")}>Side by side</button>
              <button className={`seg__btn ${mode === "diff" ? "is-active" : ""}`} onClick={() => setMode("diff")}>Differences</button>
            </div>
            <div className="row" style={{ gap: 8, alignItems: "center" }}>
              <Button size="sm" variant="secondary" disabled={page <= 1} onClick={() => setPage((p) => Math.max(1, p - 1))} leftIcon={<Icon name="chevronRight" size={14} style={{ transform: "rotate(180deg)" }} />}>Prev</Button>
              <span className="muted" style={{ fontSize: 13 }}>Page {page} / {maxPages}</span>
              <Button size="sm" variant="secondary" disabled={page >= maxPages} onClick={() => setPage((p) => Math.min(maxPages, p + 1))} rightIcon={<Icon name="chevronRight" size={14} />}>Next</Button>
            </div>
          </div>

          {mode === "diff" && diff && (
            <div className="row gap-sm mt" style={{ alignItems: "center" }}>
              <span className="cmp-legend"><span className="cmp-legend__swatch" /> changed</span>
              <span className="muted" style={{ fontSize: 13 }}>
                {diff.changedPercent < 0.01 ? "Identical pages" : `${diff.changedPercent.toFixed(2)}% of the page changed`}
              </span>
            </div>
          )}

          <div className="cmp-stage mt">
            {loading ? (
              <Spinner />
            ) : mode === "side" ? (
              <div className="cmp-side">
                <div className="cmp-pane">
                  <div className="cmp-pane__label">A · {a!.name}</div>
                  {aUrl ? <img src={aUrl} alt="" /> : <div className="cmp-blank">No page {page}</div>}
                </div>
                <div className="cmp-pane">
                  <div className="cmp-pane__label">B · {b!.name}</div>
                  {bUrl ? <img src={bUrl} alt="" /> : <div className="cmp-blank">No page {page}</div>}
                </div>
              </div>
            ) : diff ? (
              <img src={diff.dataUrl} alt="" style={{ maxWidth: "100%", borderRadius: 8 }} />
            ) : (
              <div className="cmp-blank">Page {page} doesn't exist in both files.</div>
            )}
          </div>
        </ToolSection>
      )}
    </ToolPage>
  );
}
