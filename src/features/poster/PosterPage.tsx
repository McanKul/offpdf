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
import { posterPdf, pagePdf } from "@/lib/tauriCommands";
import { getTool } from "@/lib/tools";
import { estimateRequiredBytes, validateOutputName, joinPath } from "@/lib/validation";
import { stripExt } from "@/lib/formatBytes";
import {
  PAPERS,
  PT_PER_MM,
  gridFor,
  paperPt,
  describeSize,
  type Orientation,
  type PaperId,
} from "@/lib/paper";
import { pdfjsLib, base64ToBytes, PDF_OPTS } from "@/lib/pdfjs";
import { useSettingsStore } from "@/state/settingsStore";
import { useWorkspace } from "@/state/workspaceStore";

const tool = getTool("poster");

const PAPER_OPTS = PAPERS.map((p) => ({ value: p.id, label: p.label }));
const ORIENT_OPTS = [
  { value: "auto", label: "Auto (fewest sheets)" },
  { value: "portrait", label: "Portrait" },
  { value: "landscape", label: "Landscape" },
];

export function PosterPage() {
  const files = useWorkspace((s) => s.files);
  const refs = useCombinedDoc();
  const job = useJob();
  const disk = useDiskGuard();
  const { toast } = useToast();

  const lastFolder = useSettingsStore((s) => s.lastOutputFolder);
  const [folder, setFolder] = useState<string | null>(lastFolder);
  const [name, setName] = useState("poster.pdf");
  const [page, setPage] = useState("1");
  const [paper, setPaper] = useState<PaperId>("a4");
  const [orientation, setOrientation] = useState<Orientation>("auto");
  const [overlapMm, setOverlapMm] = useState("10");
  const [marks, setMarks] = useState(true);
  const [pageSize, setPageSize] = useState<{ w: number; h: number } | null>(null);

  const total = refs.length;
  const first = files[0];
  useEffect(() => {
    if (first) setName(`${stripExt(first.name)}-poster.pdf`);
  }, [first?.path]);

  const pageNum = Number(page);
  const ref = Number.isInteger(pageNum) && pageNum >= 1 ? refs[pageNum - 1] : undefined;

  // Load the chosen page's true size (points) for a live sheet-count estimate.
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

  const overlapPt = Math.max(0, Number(overlapMm) || 0) * PT_PER_MM;
  const grid = pageSize ? gridFor(pageSize.w, pageSize.h, paper, orientation, overlapPt) : null;
  const tile = grid
    ? { w: grid.tileW, h: grid.tileH }
    : paperPt(paper, orientation !== "landscape");

  const start = async () => {
    if (refs.length === 0) return toast({ title: "Add a PDF first", variant: "error" });
    if (!Number.isInteger(pageNum) || pageNum < 1 || pageNum > total)
      return toast({ title: `Page must be between 1 and ${total}`, variant: "error" });
    if (!folder) return toast({ title: "Choose an output folder", variant: "error" });
    const nameRes = validateOutputName(name);
    if (!nameRes.ok) return toast({ title: "Invalid file name", description: nameRes.error, variant: "error" });
    const outputPath = joinPath(folder, nameRes.value);
    if (!(await disk.ensure(folder, estimateRequiredBytes("optimize", files.map((f) => f.sizeBytes))))) return;

    await job.run(
      (id) => posterPdf(id, outputPath, buildGroups(refs), pageNum, tile.w, tile.h, overlapPt, marks),
      { tool: "poster", label: grid ? `Poster ${grid.cols}×${grid.rows}` : "Poster" },
    );
  };

  const canStart = refs.length > 0 && !!folder && !job.isBusy && Number.isInteger(pageNum) && pageNum >= 1;

  return (
    <ToolPage tool={tool}>
      <ToolSection label="Document">
        <WorkspaceFilePicker selectable={false} />
        {files.length > 0 && (
          <div className="mt">
            <LargeFileWarning files={files} />
          </div>
        )}
      </ToolSection>

      <ToolSection label="Tiles">
        <div className="col">
          <div className="row">
            <Select label="Sheet size" value={paper} onChange={(v) => setPaper(v as PaperId)} options={PAPER_OPTS} />
            <Select
              label="Orientation"
              value={orientation}
              onChange={(v) => setOrientation(v as Orientation)}
              options={ORIENT_OPTS}
            />
          </div>
          <div className="row">
            <Input
              label="Page to tile"
              type="number"
              min={1}
              max={total || 1}
              value={page}
              onChange={(e) => setPage(e.target.value)}
              hint={total ? `1–${total}` : undefined}
            />
            <Input
              label="Overlap (mm)"
              type="number"
              min={0}
              max={50}
              value={overlapMm}
              onChange={(e) => setOverlapMm(e.target.value)}
              hint="Shared edge for easier taping"
            />
          </div>
          <label className="row gap-sm" style={{ cursor: "pointer", alignItems: "center" }}>
            <input type="checkbox" checked={marks} onChange={(e) => setMarks(e.target.checked)} />
            <span>Show cut lines (dashed guide on each sheet)</span>
          </label>
          {grid && pageSize && (
            <Alert variant="info">
              Your page: {describeSize(pageSize.w, pageSize.h)} → <strong>{grid.cols} × {grid.rows} = {grid.count}</strong>{" "}
              sheet{grid.count === 1 ? "" : "s"}. Print each at 100% (actual size) and tape together.
            </Alert>
          )}
        </div>
      </ToolSection>

      {refs.length > 0 && (
        <ToolSection label="Preview (page to tile)">
          <CombinedPreview />
        </ToolSection>
      )}

      <ToolSection label="Output">
        <div className="col">
          <Input label="File name" value={name} onChange={(e) => setName(e.target.value)} placeholder="poster.pdf" />
          <OutputFolderPicker value={folder} onChange={setFolder} />
        </div>
      </ToolSection>

      <div className="row">
        <Button variant="primary" size="lg" onClick={start} disabled={!canStart} loading={job.isBusy} leftIcon={<Icon name="poster" size={18} />}>
          Make poster
        </Button>
      </div>

      <JobStatus job={job} />
      {disk.modal}
    </ToolPage>
  );
}
