import { useEffect, useState } from "react";
import { ToolPage, ToolSection } from "@/components/tools/ToolPage";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Icon } from "@/components/ui/Icon";
import { Alert } from "@/components/ui/Alert";
import { Tabs } from "@/components/ui/Tabs";
import { useToast } from "@/components/ui/Toast";
import {
  WorkspaceFilePicker,
  CombinedPreview,
  OutputFolderPicker,
  LargeFileWarning,
  useCombinedDoc,
  buildPicks,
  buildGroups,
} from "@/components/pdf";
import { useJob, JobStatus, useDiskGuard } from "@/components/jobs";
import { compressPdf, optimizePdf, getFileInfo } from "@/lib/tauriCommands";
import { getTool } from "@/lib/tools";
import { estimateRequiredBytes, validateOutputName, joinPath } from "@/lib/validation";
import { stripExt, formatBytes } from "@/lib/formatBytes";
import { useSettingsStore } from "@/state/settingsStore";
import { useWorkspace } from "@/state/workspaceStore";

const tool = getTool("compress");

type Mode = "target" | "keepText";

export function CompressPage() {
  const files = useWorkspace((s) => s.files);
  const refs = useCombinedDoc();
  const job = useJob();
  const disk = useDiskGuard();
  const { toast } = useToast();

  const lastFolder = useSettingsStore((s) => s.lastOutputFolder);
  const [folder, setFolder] = useState<string | null>(lastFolder);
  const [name, setName] = useState("compressed.pdf");

  const [mode, setMode] = useState<Mode>("target");
  const [targetMb, setTargetMb] = useState("20");
  // Internal ceilings for the auto algorithm (not shown to the user). High on
  // purpose: a single page with a generous target can use full resolution/quality
  // (tens of MB), while many pages / a tight target auto-drop well below these.
  const MAX_DPI = 600;
  const MAX_QUALITY = 92;

  const totalBytes = files.reduce((a, f) => a + f.sizeBytes, 0);
  const first = files[0];

  useEffect(() => {
    if (first) setName(`${stripExt(first.name)}-${mode === "keepText" ? "optimized" : "compressed"}.pdf`);
  }, [first?.path, mode]);

  // Suggest a default target when files change: 50 MB for big files, otherwise
  // about half the current size.
  useEffect(() => {
    if (totalBytes > 0) {
      const mb = totalBytes / (1024 * 1024);
      const suggested = mb > 50 ? 50 : Math.max(1, Math.round(mb * 0.5));
      setTargetMb(String(suggested));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [first?.path]);

  const currentMb = totalBytes / (1024 * 1024);
  const targetTooBig = currentMb > 0 && Number(targetMb) >= currentMb * 0.98;

  const start = async () => {
    if (refs.length === 0) return toast({ title: "Add a PDF or image first", variant: "error" });
    if (!folder) return toast({ title: "Choose an output folder", variant: "error" });
    const nameRes = validateOutputName(name);
    if (!nameRes.ok) return toast({ title: "Invalid file name", description: nameRes.error, variant: "error" });
    const outputPath = joinPath(folder, nameRes.value);

    if (mode === "keepText") {
      if (!(await disk.ensure(folder, estimateRequiredBytes("optimize", files.map((f) => f.sizeBytes))))) return;
      await job.run((id) => optimizePdf(id, outputPath, buildGroups(refs)), {
        tool: "compress",
        label: `Optimize ${refs.length} pages`,
        inputBytes: totalBytes,
      });
      return;
    }

    // target mode
    const mb = Number(targetMb);
    if (!(mb > 0)) return toast({ title: "Enter a target size in MB", variant: "error" });
    const targetBytes = Math.round(mb * 1024 * 1024);

    if (!(await disk.ensure(folder, estimateRequiredBytes("compress", files.map((f) => f.sizeBytes))))) return;

    // Only a mild reduction requested → try the lossless cleanup first (keeps
    // text). If that already fits the target, use it; otherwise rasterize.
    const tryLosslessFirst = totalBytes > 0 && targetBytes >= totalBytes * 0.5;
    const groups = buildGroups(refs);
    const picks = buildPicks(refs);

    await job.run(
      async (id) => {
        if (tryLosslessFirst) {
          const lossless = await optimizePdf(id, outputPath, groups);
          try {
            const info = await getFileInfo(outputPath);
            if (info.sizeBytes <= targetBytes) return lossless; // lossless was enough
          } catch {
            /* fall through to lossy */
          }
        }
        return compressPdf(id, outputPath, picks, MAX_DPI, MAX_QUALITY, targetBytes);
      },
      { tool: "compress", label: `Compress to ${mb} MB`, inputBytes: totalBytes },
    );
  };

  const canStart = refs.length > 0 && !!folder && !job.isBusy;

  return (
    <ToolPage tool={tool}>
      <ToolSection label="Documents" sublabel="All loaded files become one PDF.">
        <WorkspaceFilePicker selectable={false} />
        {files.length > 0 && (
          <div className="mt">
            <LargeFileWarning files={files} />
          </div>
        )}
      </ToolSection>

      <ToolSection label="Reduce size">
        <Tabs
          tabs={[
            { id: "target", label: "To target size" },
            { id: "keepText", label: "Keep text (lossless)" },
          ]}
          active={mode}
          onChange={(t) => setMode(t as Mode)}
        />

        {mode === "target" ? (
          <div className="col mt">
            <Input
              label="Target size (MB)"
              type="number"
              min={1}
              step="any"
              value={targetMb}
              onChange={(e) => setTargetMb(e.target.value)}
              hint={
                totalBytes > 0
                  ? `Current total: ${formatBytes(totalBytes)}. OffPDF auto-tunes resolution + quality to get close.`
                  : "OffPDF auto-tunes resolution and quality to reach this size."
              }
            />
            {targetTooBig ? (
              <Alert variant="info" title="Nothing to compress at that size">
                Your target ({targetMb} MB) isn’t smaller than the current file ({formatBytes(totalBytes)}),
                so OffPDF will just do a lossless cleanup (keeps your text). Pick a smaller number to
                actually shrink it.
              </Alert>
            ) : (
              <Alert variant="info">
                OffPDF first tries a lossless cleanup (keeps text); if that already meets your target it
                stops there. Otherwise it re-renders pages as images to hit the target — smaller targets
                mean more quality loss, and resolution is lowered before quality.
              </Alert>
            )}
          </div>
        ) : (
          <div className="mt">
            <Alert variant="success" title="Keeps your text" icon="shield">
              Lossless cleanup: linearizes the file and rebuilds object streams without touching page
              content, so selectable text and vectors stay intact. Size reduction depends on the PDF.
            </Alert>
          </div>
        )}
      </ToolSection>

      {refs.length > 0 && (
        <ToolSection label="Preview">
          <CombinedPreview />
        </ToolSection>
      )}

      <ToolSection label="Output">
        <div className="col">
          <Input label="File name" value={name} onChange={(e) => setName(e.target.value)} placeholder="document-compressed.pdf" />
          <OutputFolderPicker value={folder} onChange={setFolder} />
        </div>
      </ToolSection>

      <div className="row">
        <Button variant="primary" size="lg" onClick={start} disabled={!canStart} loading={job.isBusy} leftIcon={<Icon name="compress" size={18} />}>
          {mode === "keepText" ? "Optimize PDF" : "Compress PDF"}
        </Button>
      </div>

      <JobStatus job={job} />
      {disk.modal}
    </ToolPage>
  );
}
