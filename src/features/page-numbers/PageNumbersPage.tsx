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
import { getTool } from "@/lib/tools";
import { addPageNumbers } from "@/lib/tauriCommands";
import { estimateRequiredBytes, validateOutputName, joinPath } from "@/lib/validation";
import { stripExt } from "@/lib/formatBytes";
import { useSettingsStore } from "@/state/settingsStore";
import { useWorkspace } from "@/state/workspaceStore";

const tool = getTool("pageNumbers");

const POSITIONS = [
  { value: "bottom-center", label: "Bottom center" },
  { value: "bottom-right", label: "Bottom right" },
  { value: "bottom-left", label: "Bottom left" },
  { value: "top-center", label: "Top center" },
  { value: "top-right", label: "Top right" },
  { value: "top-left", label: "Top left" },
];

/** Label format: plain numbers, or Bates-style prefix + zero-padded counter,
 * optionally followed by today's date. */
type FormatMode = "plain" | "bates" | "bates-date";

const FORMATS = [
  { value: "plain", label: "Plain number (1, 2, 3…)" },
  { value: "bates", label: "Prefix + padded (DAVA-000123)" },
  { value: "bates-date", label: "Prefix + padded + date" },
];

/** Live example of the first page's label, mirroring the backend format. */
function previewLabel(fm: FormatMode, prefix: string, pad: number, start: number): string {
  if (fm === "plain") return String(start);
  const padded = String(Math.abs(start)).padStart(pad, "0");
  const num = `${start < 0 ? "-" : ""}${padded}`;
  const date = fm === "bates-date" ? ` – ${new Date().toLocaleDateString("tr-TR")}` : "";
  return `${prefix}${num}${date}`;
}

export function PageNumbersPage() {
  const files = useWorkspace((s) => s.files);
  const refs = useCombinedDoc();
  const job = useJob();
  const disk = useDiskGuard();
  const { toast } = useToast();

  const lastFolder = useSettingsStore((s) => s.lastOutputFolder);
  const [folder, setFolder] = useState<string | null>(lastFolder);
  const [name, setName] = useState("numbered.pdf");
  const [position, setPosition] = useState("bottom-center");
  const [start, setStart] = useState("1");
  const [formatMode, setFormatMode] = useState<FormatMode>("plain");
  const [prefix, setPrefix] = useState("");
  const [padWidth, setPadWidth] = useState("6");

  const first = files[0];
  useEffect(() => {
    if (first) setName(`${stripExt(first.name)}-numbered.pdf`);
  }, [first?.path]);

  const run = async () => {
    if (refs.length === 0) return toast({ title: "Add a PDF first", variant: "error" });
    if (!folder) return toast({ title: "Choose an output folder", variant: "error" });
    const startNum = Number(start);
    if (!Number.isInteger(startNum)) return toast({ title: "Start number must be a whole number", variant: "error" });
    const padNum = Number(padWidth);
    const isBates = formatMode !== "plain";
    if (isBates && (!Number.isInteger(padNum) || padNum < 0 || padNum > 12))
      return toast({ title: "Digits must be a whole number between 0 and 12", variant: "error" });
    const nameRes = validateOutputName(name);
    if (!nameRes.ok) return toast({ title: "Invalid file name", description: nameRes.error, variant: "error" });
    const outputPath = joinPath(folder, nameRes.value);
    if (!(await disk.ensure(folder, estimateRequiredBytes("optimize", files.map((f) => f.sizeBytes))))) return;

    await job.run(
      (id) =>
        addPageNumbers(
          id,
          outputPath,
          buildGroups(refs),
          position,
          startNum,
          isBates ? prefix : undefined,
          isBates ? padNum : undefined,
          formatMode === "bates-date" ? true : undefined,
        ),
      {
        tool: "pageNumbers",
        label: `Number ${refs.length} pages`,
      },
    );
  };

  const canStart = refs.length > 0 && !!folder && !job.isBusy;

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

      <ToolSection label="Numbering">
        <div className="row">
          <Select label="Position" value={position} onChange={setPosition} options={POSITIONS} />
          <Input label="Start at" type="number" value={start} onChange={(e) => setStart(e.target.value)} hint="First page's number." />
        </div>
      </ToolSection>

      <ToolSection label="Format" sublabel="Bates numbering for legal/archive filing — e.g. DAVA-000123.">
        <div className="col">
          <Select
            label="Label format"
            value={formatMode}
            onChange={(v) => setFormatMode(v as FormatMode)}
            options={FORMATS}
          />
          {formatMode !== "plain" && (
            <div className="row">
              <Input
                label="Prefix"
                value={prefix}
                onChange={(e) => setPrefix(e.target.value)}
                placeholder="DAVA-"
                hint="Turkish characters (İ ş ğ …) are fine."
              />
              <Input
                label="Digits"
                type="number"
                min={0}
                max={12}
                value={padWidth}
                onChange={(e) => setPadWidth(e.target.value)}
                hint="Counter is zero-padded to this width."
              />
            </div>
          )}
          <Alert variant="info">
            First page will read: <strong>{previewLabel(formatMode, prefix, Math.max(0, Number(padWidth) || 0), Number(start) || 1)}</strong>
          </Alert>
        </div>
      </ToolSection>

      {refs.length > 0 && (
        <ToolSection label="Preview">
          <CombinedPreview />
        </ToolSection>
      )}

      <ToolSection label="Output">
        <div className="col">
          <Input label="File name" value={name} onChange={(e) => setName(e.target.value)} placeholder="numbered.pdf" />
          <OutputFolderPicker value={folder} onChange={setFolder} />
        </div>
      </ToolSection>

      <div className="row">
        <Button variant="primary" size="lg" onClick={run} disabled={!canStart} loading={job.isBusy} leftIcon={<Icon name="hash" size={18} />}>
          Add page numbers
        </Button>
      </div>

      <JobStatus job={job} />
      {disk.modal}
    </ToolPage>
  );
}
