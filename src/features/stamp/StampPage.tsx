import { useEffect, useState } from "react";
import { ToolPage, ToolSection } from "@/components/tools/ToolPage";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Select } from "@/components/ui/Select";
import { Icon } from "@/components/ui/Icon";
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
import { stampPdf } from "@/lib/tauriCommands";
import { getTool } from "@/lib/tools";
import { estimateRequiredBytes, validateOutputName, joinPath } from "@/lib/validation";
import { stripExt } from "@/lib/formatBytes";
import { useSettingsStore } from "@/state/settingsStore";
import { useWorkspace } from "@/state/workspaceStore";

const tool = getTool("stamp");

const ANCHORS = [
  "top-left", "top-center", "top-right",
  "middle-left", "middle-center", "middle-right",
  "bottom-left", "bottom-center", "bottom-right",
];

const COLORS: { label: string; value: [number, number, number] }[] = [
  { label: "Black", value: [0, 0, 0] },
  { label: "Red", value: [0.85, 0.12, 0.12] },
  { label: "Blue", value: [0.1, 0.3, 0.8] },
  { label: "Gray", value: [0.45, 0.45, 0.45] },
];

const SIZES = [
  { value: "0.025", label: "Small" },
  { value: "0.04", label: "Medium" },
  { value: "0.06", label: "Large" },
];

export function StampPage() {
  const files = useWorkspace((s) => s.files);
  const refs = useCombinedDoc();
  const job = useJob();
  const disk = useDiskGuard();
  const { toast } = useToast();

  const lastFolder = useSettingsStore((s) => s.lastOutputFolder);
  const [folder, setFolder] = useState<string | null>(lastFolder);
  const [name, setName] = useState("stamped.pdf");
  const [text, setText] = useState("APPROVED");
  const [anchor, setAnchor] = useState("bottom-right");
  const [colorIdx, setColorIdx] = useState(1);
  const [size, setSize] = useState("0.04");
  const [page, setPage] = useState("1");

  const total = refs.length;
  const first = files[0];
  useEffect(() => {
    if (first) setName(`${stripExt(first.name)}-stamped.pdf`);
  }, [first?.path]);

  const start = async () => {
    if (refs.length === 0) return toast({ title: "Add a PDF first", variant: "error" });
    if (!text.trim()) return toast({ title: "Type the stamp text", variant: "error" });
    if (!folder) return toast({ title: "Choose an output folder", variant: "error" });
    const pageNum = Number(page);
    if (!Number.isInteger(pageNum) || pageNum < 1 || pageNum > total)
      return toast({ title: `Page must be between 1 and ${total}`, variant: "error" });
    const nameRes = validateOutputName(name);
    if (!nameRes.ok) return toast({ title: "Invalid file name", description: nameRes.error, variant: "error" });
    const outputPath = joinPath(folder, nameRes.value);
    if (!(await disk.ensure(folder, estimateRequiredBytes("optimize", files.map((f) => f.sizeBytes))))) return;

    await job.run((id) => stampPdf(id, outputPath, buildGroups(refs), pageNum, anchor, text, COLORS[colorIdx].value, Number(size)), {
      tool: "stamp",
      label: `Stamp page ${pageNum}`,
    });
  };

  const canStart = refs.length > 0 && !!folder && text.trim().length > 0 && !job.isBusy;

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

      <ToolSection label="Stamp">
        <div className="col">
          <div className="row">
            <Input label="Text" value={text} onChange={(e) => setText(e.target.value)} placeholder="e.g. APPROVED, your name, a date" />
            <Input label="Page" type="number" min={1} max={total || 1} value={page} onChange={(e) => setPage(e.target.value)} hint={total ? `1–${total}` : undefined} />
          </div>
          <div className="row" style={{ alignItems: "flex-end" }}>
            <Select label="Size" value={size} onChange={setSize} options={SIZES} />
            <div className="field">
              <label className="field__label">Color</label>
              <div className="row" style={{ gap: 6 }}>
                {COLORS.map((c, i) => (
                  <button
                    key={c.label}
                    title={c.label}
                    onClick={() => setColorIdx(i)}
                    style={{
                      width: 26, height: 26, borderRadius: 7, cursor: "pointer",
                      border: i === colorIdx ? "2px solid var(--text)" : "2px solid var(--border)",
                      background: `rgb(${c.value.map((v) => Math.round(v * 255)).join(",")})`,
                    }}
                  />
                ))}
              </div>
            </div>
          </div>
          <div className="field">
            <label className="field__label">Position on the page</label>
            <div className="anchor-grid">
              {ANCHORS.map((a) => (
                <button key={a} className={`anchor-cell ${a === anchor ? "is-active" : ""}`} onClick={() => setAnchor(a)} title={a} />
              ))}
            </div>
          </div>
        </div>
      </ToolSection>

      {refs.length > 0 && (
        <ToolSection label="Preview (stamp added on output)">
          <CombinedPreview />
        </ToolSection>
      )}

      <ToolSection label="Output">
        <div className="col">
          <Input label="File name" value={name} onChange={(e) => setName(e.target.value)} placeholder="stamped.pdf" />
          <OutputFolderPicker value={folder} onChange={setFolder} />
        </div>
      </ToolSection>

      <div className="row">
        <Button variant="primary" size="lg" onClick={start} disabled={!canStart} loading={job.isBusy} leftIcon={<Icon name="stamp" size={18} />}>
          Add stamp
        </Button>
      </div>

      <JobStatus job={job} />
      {disk.modal}
    </ToolPage>
  );
}
