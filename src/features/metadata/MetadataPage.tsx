import { useEffect, useState } from "react";
import { ToolPage, ToolSection } from "@/components/tools/ToolPage";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Icon } from "@/components/ui/Icon";
import { Alert } from "@/components/ui/Alert";
import { Spinner } from "@/components/ui/Spinner";
import { useToast } from "@/components/ui/Toast";
import { WorkspaceFilePicker, OutputFolderPicker } from "@/components/pdf";
import { useJob, JobStatus, useDiskGuard } from "@/components/jobs";
// `readPdfMeta`, `writePdfMeta` + the `PdfMeta` type are added by
// INTEGRATION-utils.md (tauriCommands.ts).
import { readPdfMeta, writePdfMeta, type PdfMeta } from "@/lib/tauriCommands";
import { getTool } from "@/lib/tools";
import { toAppError } from "@/lib/types";
import { estimateRequiredBytes, validateOutputName, joinPath } from "@/lib/validation";
import { stripExt } from "@/lib/formatBytes";
import { useSettingsStore } from "@/state/settingsStore";
import { useWorkspace } from "@/state/workspaceStore";

const tool = getTool("metadata");

const EMPTY: PdfMeta = {
  title: null,
  author: null,
  subject: null,
  keywords: null,
  creator: null,
  producer: null,
};

const FIELDS: { key: keyof PdfMeta; label: string; hint?: string }[] = [
  { key: "title", label: "Title" },
  { key: "author", label: "Author" },
  { key: "subject", label: "Subject" },
  { key: "keywords", label: "Keywords", hint: "Comma-separated, e.g. invoice, 2026, draft" },
  { key: "creator", label: "Creator", hint: "The application that created the original document." },
  { key: "producer", label: "Producer", hint: "The application that produced this PDF file." },
];

export function MetadataPage() {
  const files = useWorkspace((s) => s.files);
  const job = useJob();
  const disk = useDiskGuard();
  const { toast } = useToast();

  const lastFolder = useSettingsStore((s) => s.lastOutputFolder);
  const [folder, setFolder] = useState<string | null>(lastFolder);
  const [name, setName] = useState("metadata.pdf");

  const [meta, setMeta] = useState<PdfMeta>(EMPTY);
  const [clearAll, setClearAll] = useState(false);
  const [isLoading, setIsLoading] = useState(false);

  const first = files[0];

  // Load the current /Info values whenever the (first) file changes.
  useEffect(() => {
    if (!first) {
      setMeta(EMPTY);
      return;
    }
    setName(`${stripExt(first.name)}-metadata.pdf`);
    setClearAll(false);
    setIsLoading(true);
    let stale = false;
    readPdfMeta(first.path)
      .then((m: PdfMeta) => {
        if (!stale) setMeta({ ...EMPTY, ...m });
      })
      .catch((e: unknown) => {
        if (stale) return;
        setMeta(EMPTY);
        const err = toAppError(e);
        toast({ title: err.title, description: err.message, variant: "error" });
      })
      .finally(() => {
        if (!stale) setIsLoading(false);
      });
    return () => {
      stale = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [first?.path]);

  const setField = (key: keyof PdfMeta, value: string) =>
    setMeta((prev: PdfMeta) => ({ ...prev, [key]: value }));

  const save = async () => {
    if (!first) return toast({ title: "Add a PDF first", variant: "error" });
    if (!folder) return toast({ title: "Choose an output folder", variant: "error" });
    const nameRes = validateOutputName(name);
    if (!nameRes.ok) return toast({ title: "Invalid file name", description: nameRes.error, variant: "error" });
    const outputPath = joinPath(folder, nameRes.value);
    if (!(await disk.ensure(folder, estimateRequiredBytes("optimize", [first.sizeBytes])))) return;

    await job.run((id) => writePdfMeta(id, first.path, outputPath, clearAll ? EMPTY : meta, clearAll), {
      tool: "metadata",
      label: clearAll ? `Clear metadata of ${first.name}` : `Edit metadata of ${first.name}`,
    });
  };

  const canSave = !!first && !!folder && !isLoading && !job.isBusy;

  return (
    <ToolPage tool={tool}>
      <ToolSection label="Document" sublabel="Metadata is edited on one file at a time.">
        <WorkspaceFilePicker selectable={false} />
        {files.length > 1 && (
          <div className="mt">
            <Alert variant="info">
              Several files are loaded — only the first one ({first?.name}) is edited here.
            </Alert>
          </div>
        )}
      </ToolSection>

      <ToolSection label="Metadata" sublabel="Empty fields are removed from the document.">
        {isLoading ? (
          <div className="row">
            <Spinner />
            <span className="muted" style={{ fontSize: 12.5 }}>Reading current metadata…</span>
          </div>
        ) : (
          <div className="col">
            {FIELDS.map((f) => (
              <Input
                key={String(f.key)}
                label={f.label}
                hint={f.hint}
                value={meta[f.key] ?? ""}
                onChange={(e) => setField(f.key, e.target.value)}
                disabled={clearAll || !first}
              />
            ))}
            <label className="row" style={{ gap: 8, cursor: "pointer" }}>
              <input
                type="checkbox"
                checked={clearAll}
                onChange={(e) => setClearAll(e.target.checked)}
              />
              Clear all metadata (sanitize)
            </label>
            {clearAll && (
              <Alert variant="info" icon="shield">
                The entire Info dictionary is stripped and replaced with an empty one, and the file is
                rewritten so the old values are not recoverable from it. Turkish and other non-ASCII
                text is otherwise stored as UTF-16, so “Başlık Ğüzel” style titles survive editing.
              </Alert>
            )}
          </div>
        )}
      </ToolSection>

      <ToolSection label="Output">
        <div className="col">
          <Input label="File name" value={name} onChange={(e) => setName(e.target.value)} placeholder="document-metadata.pdf" />
          <OutputFolderPicker value={folder} onChange={setFolder} />
        </div>
      </ToolSection>

      <div className="row">
        <Button
          variant="primary"
          size="lg"
          onClick={save}
          disabled={!canSave}
          loading={job.isBusy}
          leftIcon={<Icon name="fileText" size={18} />}
        >
          {clearAll ? "Clear metadata" : "Save metadata"}
        </Button>
      </div>

      <JobStatus job={job} />
      {disk.modal}
    </ToolPage>
  );
}
