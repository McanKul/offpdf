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
  buildPicks,
} from "@/components/pdf";
import { useJob, JobStatus, useDiskGuard } from "@/components/jobs";
import { ocrPdf, ocrAvailable } from "@/lib/tauriCommands";
import { getTool } from "@/lib/tools";
import { estimateRequiredBytes, validateOutputName, joinPath } from "@/lib/validation";
import { stripExt } from "@/lib/formatBytes";
import { useSettingsStore } from "@/state/settingsStore";
import { useWorkspace } from "@/state/workspaceStore";

const tool = getTool("ocr");

const LANGS = [
  { value: "eng", label: "English" },
  { value: "tur", label: "Turkish" },
  { value: "eng+tur", label: "English + Turkish" },
  { value: "deu", label: "German" },
  { value: "fra", label: "French" },
];

export function OcrPage() {
  const files = useWorkspace((s) => s.files);
  const refs = useCombinedDoc();
  const job = useJob();
  const disk = useDiskGuard();
  const { toast } = useToast();

  const lastFolder = useSettingsStore((s) => s.lastOutputFolder);
  const [folder, setFolder] = useState<string | null>(lastFolder);
  const [name, setName] = useState("searchable.pdf");
  const [lang, setLang] = useState("eng");
  const [available, setAvailable] = useState(true);

  const first = files[0];
  useEffect(() => {
    if (first) setName(`${stripExt(first.name)}-ocr.pdf`);
  }, [first?.path]);
  useEffect(() => {
    let on = true;
    ocrAvailable().then((v) => on && setAvailable(v)).catch(() => {});
    return () => {
      on = false;
    };
  }, []);

  const start = async () => {
    if (refs.length === 0) return toast({ title: "Add a PDF first", variant: "error" });
    if (!folder) return toast({ title: "Choose an output folder", variant: "error" });
    const nameRes = validateOutputName(name);
    if (!nameRes.ok) return toast({ title: "Invalid file name", description: nameRes.error, variant: "error" });
    const outputPath = joinPath(folder, nameRes.value);
    if (!(await disk.ensure(folder, estimateRequiredBytes("compress", files.map((f) => f.sizeBytes))))) return;

    await job.run((id) => ocrPdf(id, outputPath, buildPicks(refs), lang), {
      tool: "ocr",
      label: `OCR ${refs.length} pages`,
    });
  };

  const canStart = refs.length > 0 && !!folder && available && !job.isBusy;

  return (
    <ToolPage tool={tool}>
      {!available && (
        <Alert variant="warning" title="Tesseract needed">
          OCR uses Tesseract, which wasn’t found. Install it (macOS:{" "}
          <span className="mono">brew install tesseract tesseract-lang</span>), then reopen this tool.
        </Alert>
      )}

      <ToolSection label="Documents">
        <WorkspaceFilePicker selectable={false} />
        {files.length > 0 && (
          <div className="mt">
            <LargeFileWarning files={files} />
          </div>
        )}
      </ToolSection>

      <ToolSection label="Language">
        <Select label="Document language" value={lang} onChange={setLang} options={LANGS} />
        <div className="mt">
          <Alert variant="info">
            Pages are rendered to images and a searchable text layer is added on top. Pick the language
            that matches the document for the best accuracy.
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
          <Input label="File name" value={name} onChange={(e) => setName(e.target.value)} placeholder="searchable.pdf" />
          <OutputFolderPicker value={folder} onChange={setFolder} />
        </div>
      </ToolSection>

      <div className="row">
        <Button variant="primary" size="lg" onClick={start} disabled={!canStart} loading={job.isBusy} leftIcon={<Icon name="scanText" size={18} />}>
          Make searchable
        </Button>
      </div>

      <JobStatus job={job} />
      {disk.modal}
    </ToolPage>
  );
}
