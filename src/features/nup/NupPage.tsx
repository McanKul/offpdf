import { useEffect, useState } from "react";
import { ToolPage, ToolSection } from "@/components/tools/ToolPage";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Select } from "@/components/ui/Select";
import { Icon } from "@/components/ui/Icon";
import { Alert } from "@/components/ui/Alert";
import { useToast } from "@/components/ui/Toast";
import {
  WorkspaceFilePicker,
  CombinedPreview,
  OutputFolderPicker,
  LargeFileWarning,
  useCombinedDoc,
  buildGroups,
} from "@/components/pdf";
import { useJob, JobStatus, useDiskGuard } from "@/components/jobs";
import { nupPdf, pagePdf } from "@/lib/tauriCommands";
import { getTool } from "@/lib/tools";
import { estimateRequiredBytes, validateOutputName, joinPath } from "@/lib/validation";
import { stripExt } from "@/lib/formatBytes";
import { PAPERS, paperPt, type Orientation, type PaperId } from "@/lib/paper";
import { pdfjsLib, base64ToBytes, PDF_OPTS } from "@/lib/pdfjs";
import { useSettingsStore } from "@/state/settingsStore";
import { useWorkspace } from "@/state/workspaceStore";

const tool = getTool("nup");

type NupMode = "2up" | "4up" | "booklet";

const MODE_OPTS = [
  { value: "2up", label: "2 pages per sheet" },
  { value: "4up", label: "4 pages per sheet" },
  { value: "booklet", label: "Booklet (fold & staple)" },
];
const PAPER_OPTS = PAPERS.map((p) => ({ value: p.id, label: p.label }));
const ORIENT_OPTS = [
  { value: "auto", label: "Auto (best fit)" },
  { value: "portrait", label: "Portrait" },
  { value: "landscape", label: "Landscape" },
];

/** Sheets produced for `n` pages in `mode` (booklet counts printed sides). */
export function sheetEstimate(mode: NupMode, n: number): { sheets: number; physical?: number } {
  if (n <= 0) return { sheets: 0 };
  if (mode === "2up") return { sheets: Math.ceil(n / 2) };
  if (mode === "4up") return { sheets: Math.ceil(n / 4) };
  const padded = Math.ceil(n / 4) * 4; // booklet pads with blanks
  return { sheets: padded / 2, physical: padded / 4 };
}

/** Sheet orientation: 2-up/booklet want the opposite of the source page's
 * orientation (two portrait pages sit side by side on a landscape sheet);
 * 4-up keeps it (a 2×2 grid preserves the aspect). */
export function autoSheetPortrait(mode: NupMode, srcPortrait: boolean): boolean {
  return mode === "4up" ? srcPortrait : !srcPortrait;
}

export function NupPage() {
  const files = useWorkspace((s) => s.files);
  const refs = useCombinedDoc();
  const job = useJob();
  const disk = useDiskGuard();
  const { toast } = useToast();

  const lastFolder = useSettingsStore((s) => s.lastOutputFolder);
  const [folder, setFolder] = useState<string | null>(lastFolder);
  const [name, setName] = useState("layout.pdf");
  const [mode, setMode] = useState<NupMode>("2up");
  const [paper, setPaper] = useState<PaperId>("a4");
  const [orientation, setOrientation] = useState<Orientation>("auto");
  const [pageSize, setPageSize] = useState<{ w: number; h: number } | null>(null);

  const first = files[0];
  useEffect(() => {
    if (first) setName(`${stripExt(first.name)}-${mode === "booklet" ? "booklet" : mode}.pdf`);
  }, [first?.path, mode]);

  // First page's displayed size, to auto-pick the sheet orientation.
  const ref = refs[0];
  useEffect(() => {
    let cancelled = false;
    if (!ref) {
      setPageSize(null);
      return;
    }
    (async () => {
      try {
        const b64 = await pagePdf(ref.path, ref.page);
        if (!b64) {
          if (!cancelled) setPageSize(null);
          return;
        }
        const data = base64ToBytes(b64);
        const pdf = await pdfjsLib.getDocument({ data, ...PDF_OPTS }).promise;
        const p = await pdf.getPage(1);
        const vp = p.getViewport({ scale: 1 });
        if (!cancelled) setPageSize({ w: vp.width, h: vp.height });
        await pdf.destroy();
      } catch {
        if (!cancelled) setPageSize(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [ref?.key]);

  const srcPortrait = pageSize ? pageSize.h >= pageSize.w : true;
  const sheetPortrait =
    orientation === "auto" ? autoSheetPortrait(mode, srcPortrait) : orientation === "portrait";
  const sheet = paperPt(paper, sheetPortrait);

  const n = refs.length;
  const est = sheetEstimate(mode, n);

  const start = async () => {
    if (n === 0) return toast({ title: "Add a PDF first", variant: "error" });
    if (mode === "booklet" && n < 3)
      return toast({ title: "A booklet needs at least 3 pages", variant: "error" });
    if (!folder) return toast({ title: "Choose an output folder", variant: "error" });
    const nameRes = validateOutputName(name);
    if (!nameRes.ok) return toast({ title: "Invalid file name", description: nameRes.error, variant: "error" });
    const outputPath = joinPath(folder, nameRes.value);
    if (!(await disk.ensure(folder, estimateRequiredBytes("optimize", files.map((f) => f.sizeBytes))))) return;

    await job.run((id) => nupPdf(id, outputPath, buildGroups(refs), mode, sheet.w, sheet.h), {
      tool: "nup",
      label: mode === "booklet" ? `Booklet (${est.sheets} sides)` : `${mode} · ${est.sheets} sheets`,
    });
  };

  const canStart = n > 0 && !!folder && !job.isBusy;

  return (
    <ToolPage tool={tool}>
      <ToolSection label="Documents">
        <WorkspaceFilePicker selectable={false} />
        {files.length > 0 && (
          <div className="mt">
            <LargeFileWarning files={files} />
          </div>
        )}
      </ToolSection>

      <ToolSection label="Layout">
        <div className="col">
          <div className="row">
            <Select label="Layout" value={mode} onChange={(v) => setMode(v as NupMode)} options={MODE_OPTS} />
            <Select label="Sheet size" value={paper} onChange={(v) => setPaper(v as PaperId)} options={PAPER_OPTS} />
            <Select
              label="Orientation"
              value={orientation}
              onChange={(v) => setOrientation(v as Orientation)}
              options={ORIENT_OPTS}
            />
          </div>
          {n > 0 && (
            <Alert variant="info">
              {mode === "booklet" ? (
                <>
                  <strong>{n}</strong> pages → <strong>{est.sheets}</strong> printed sides (
                  {est.physical} sheet{est.physical === 1 ? "" : "s"} of paper). Print double-sided
                  (flip on the short edge), fold the stack in half and staple the spine.
                </>
              ) : (
                <>
                  <strong>{n}</strong> pages → <strong>{est.sheets}</strong> sheet
                  {est.sheets === 1 ? "" : "s"} ({mode === "2up" ? "2" : "4"} per sheet, scaled to
                  fit, aspect kept).
                </>
              )}
            </Alert>
          )}
        </div>
      </ToolSection>

      {n > 0 && (
        <ToolSection label="Preview (page order)">
          <CombinedPreview />
        </ToolSection>
      )}

      <ToolSection label="Output">
        <div className="col">
          <Input label="File name" value={name} onChange={(e) => setName(e.target.value)} placeholder="layout.pdf" />
          <OutputFolderPicker value={folder} onChange={setFolder} />
        </div>
      </ToolSection>

      <div className="row">
        <Button variant="primary" size="lg" onClick={start} disabled={!canStart} loading={job.isBusy} leftIcon={<Icon name="grip" size={18} />}>
          {mode === "booklet" ? "Make booklet" : "Make print layout"}
        </Button>
      </div>

      <JobStatus job={job} />
      {disk.modal}
    </ToolPage>
  );
}
