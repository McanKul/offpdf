import { useEffect, useState } from "react";
import { ToolPage, ToolSection } from "@/components/tools/ToolPage";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
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
import { ocrPdf, ocrAvailable, ocrListLangs } from "@/lib/tauriCommands";
import { documentOcrLangs, joinOcrLangs, ocrLangLabel } from "@/lib/ocrLangs";
import { getTool } from "@/lib/tools";
import { toAppError } from "@/lib/types";
import { estimateRequiredBytes, validateOutputName, joinPath } from "@/lib/validation";
import { stripExt } from "@/lib/formatBytes";
import { useSettingsStore } from "@/state/settingsStore";
import { useWorkspace } from "@/state/workspaceStore";

const tool = getTool("ocr");

export function OcrPage() {
  const files = useWorkspace((s) => s.files);
  const refs = useCombinedDoc();
  const job = useJob();
  const disk = useDiskGuard();
  const { toast } = useToast();

  const lastFolder = useSettingsStore((s) => s.lastOutputFolder);
  const [folder, setFolder] = useState<string | null>(lastFolder);
  const [name, setName] = useState("searchable.pdf");
  const [available, setAvailable] = useState(true);
  const [installed, setInstalled] = useState<string[] | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string[]>([]);

  const first = files[0];
  useEffect(() => {
    if (first) setName(`${stripExt(first.name)}-ocr.pdf`);
  }, [first?.path]);
  useEffect(() => {
    let on = true;
    ocrAvailable()
      .then((v) => {
        if (on) setAvailable(v);
      })
      .catch(() => {});
    return () => {
      on = false;
    };
  }, []);
  useEffect(() => {
    let on = true;
    ocrListLangs()
      .then((codes) => {
        if (!on) return;
        setInstalled(codes);
        setListError(null);
        setSelected(documentOcrLangs(codes).includes("eng") ? ["eng"] : []);
      })
      .catch((e) => {
        if (!on) return;
        setInstalled(null);
        setListError(toAppError(e).message);
        setSelected([]);
      });
    return () => {
      on = false;
    };
  }, []);

  const toggle = (code: string, on: boolean) => {
    setSelected((prev) => (on ? [...prev, code] : prev.filter((c) => c !== code)));
  };

  const start = async () => {
    if (refs.length === 0) return toast({ title: "Add a PDF first", variant: "error" });
    if (!folder) return toast({ title: "Choose an output folder", variant: "error" });
    if (!installed) {
      return toast({
        title: "Languages unavailable",
        description: listError ?? "Could not list installed OCR languages.",
        variant: "error",
      });
    }
    const lang = joinOcrLangs(selected);
    if (!lang) return toast({ title: "Select a language", variant: "error" });
    const missing = selected.filter((c) => !installed.includes(c));
    if (missing.length > 0) {
      return toast({
        title: "Language pack not installed",
        description: `“${missing[0]}” is not installed for this Tesseract.`,
        variant: "error",
      });
    }
    const nameRes = validateOutputName(name);
    if (!nameRes.ok) return toast({ title: "Invalid file name", description: nameRes.error, variant: "error" });
    const outputPath = joinPath(folder, nameRes.value);
    if (!(await disk.ensure(folder, estimateRequiredBytes("compress", files.map((f) => f.sizeBytes))))) return;

    await job.run((id) => ocrPdf(id, outputPath, buildPicks(refs), lang), {
      tool: "ocr",
      label: `OCR ${refs.length} pages`,
    });
  };

  const documentLangs = installed === null ? [] : documentOcrLangs(installed);
  const listedOk = installed !== null && documentLangs.length > 0;
  const canStart = refs.length > 0 && !!folder && available && !job.isBusy && selected.length > 0 && listedOk;

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
        {listError && available && (
          <Alert variant="danger" title="Could not list OCR languages">
            {listError}
          </Alert>
        )}
        {installed && documentLangs.length > 0 && (
          <div className="col" role="group" aria-label="Document languages">
            {documentLangs.map((code) => (
              <label key={code} className="row" style={{ gap: 8, cursor: "pointer", alignItems: "center" }}>
                <input
                  type="checkbox"
                  checked={selected.includes(code)}
                  onChange={(e) => toggle(code, e.target.checked)}
                />
                <span>{ocrLangLabel(code)}</span>
              </label>
            ))}
          </div>
        )}
        {available && installed === null && !listError && (
          <p className="muted">Loading languages…</p>
        )}
        {installed && !listError && documentLangs.length === 0 && (
          <p className="muted">
            No document language packs. Install tesseract-lang (OffPDF does not download them).
          </p>
        )}
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
