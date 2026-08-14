import { useEffect, useMemo, useRef, useState } from "react";
import { ToolPage, ToolSection } from "@/components/tools/ToolPage";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Icon } from "@/components/ui/Icon";
import { Modal } from "@/components/ui/Modal";
import { useToast } from "@/components/ui/Toast";
import {
  WorkspaceFilePicker,
  OutputFolderPicker,
  LargeFileWarning,
  useCombinedDoc,
  buildGroups,
  pageKeysForFiles,
  PdfEditorCanvas,
} from "@/components/pdf";
import { useEditSession } from "@/components/pdf/editor";
import { useJob, JobStatus, useDiskGuard } from "@/components/jobs";
import { editPdfOverlays } from "@/lib/tauriCommands";
import { getTool } from "@/lib/tools";
import { estimateRequiredBytes, validateOutputName, joinPath } from "@/lib/validation";
import { stripExt } from "@/lib/formatBytes";
import { useSettingsStore } from "@/state/settingsStore";
import { useWorkspace } from "@/state/workspaceStore";
import {
  clampPageIndex,
  planKeyRebind,
  rebindNeedsConfirm,
  resolveViewPageIndex,
  samePageKeys,
  shouldShowEditCanvas,
  toExportDocument,
} from "@/lib/editor";
import { isTauriRuntime } from "@/lib/tauriEnv";
import { Alert } from "@/components/ui/Alert";

const tool = getTool("editPdf");

export function EditPdfPage() {
  const files = useWorkspace((s) => s.files);
  const refs = useCombinedDoc();
  const job = useJob();
  const disk = useDiskGuard();
  const { toast } = useToast();
  const inTauri = isTauriRuntime();

  const lastFolder = useSettingsStore((s) => s.lastOutputFolder);
  const [folder, setFolder] = useState<string | null>(lastFolder);
  const [name, setName] = useState("edited.pdf");
  const [pageIndex, setPageIndex] = useState(0);
  const [discardPrompt, setDiscardPrompt] = useState<{ count: number; historyOnly: boolean } | null>(null);
  const discardResolve = useRef<((ok: boolean) => void) | undefined>(undefined);
  const prevKeyRef = useRef<string | undefined>(undefined);
  const prevKeysRef = useRef<string[]>([]);
  const pageKeys = useMemo(() => refs.map((r) => r.key), [refs]);
  let viewIndex = pageIndex;
  if (!samePageKeys(prevKeysRef.current, pageKeys)) {
    viewIndex = resolveViewPageIndex(pageKeys, pageIndex, prevKeyRef.current);
    prevKeysRef.current = pageKeys;
  }
  viewIndex = clampPageIndex(viewIndex, refs.length);
  if (viewIndex !== pageIndex) setPageIndex(viewIndex);
  prevKeyRef.current = pageKeys[viewIndex];

  const session = useEditSession(pageKeys);
  const doc = session.document;

  const first = files[0];
  useEffect(() => {
    if (first) setName(`${stripExt(first.name)}-edited.pdf`);
  }, [first?.path]);

  useEffect(
    () => () => {
      discardResolve.current?.(false);
      discardResolve.current = undefined;
    },
    [],
  );

  const askDiscard = (count: number, historyOnly: boolean) =>
    new Promise<boolean>((resolve) => {
      discardResolve.current = resolve;
      setDiscardPrompt({ count, historyOnly });
    });

  const closeDiscard = (ok: boolean) => {
    setDiscardPrompt(null);
    discardResolve.current?.(ok);
    discardResolve.current = undefined;
  };

  const onBeforeRemove = async (index: number) => {
    const nextKeys = pageKeysForFiles(files.filter((_, i) => i !== index));
    const plan = planKeyRebind(session.document, session.past, session.future, pageKeys, nextKeys);
    if (!plan || !rebindNeedsConfirm(plan)) return true;
    return askDiscard(plan.droppedIds.length, plan.droppedIds.length === 0 && plan.historyDropped);
  };

  const current = refs[viewIndex];
  const editCanvas = shouldShowEditCanvas(files.length, refs.length, inTauri);

  const start = async () => {
    if (refs.length === 0) return toast({ title: "Add a PDF first", variant: "error" });
    if (doc.objects.length === 0) return toast({ title: "Add something to the page first", variant: "error" });
    if (!folder) return toast({ title: "Choose an output folder", variant: "error" });
    const nameRes = validateOutputName(name);
    if (!nameRes.ok) return toast({ title: "Invalid file name", description: nameRes.error, variant: "error" });
    const outputPath = joinPath(folder, nameRes.value);
    if (files.some((f) => f.path === outputPath)) {
      return toast({ title: "Choose a new file name", description: "The original file is never overwritten.", variant: "error" });
    }
    if (!(await disk.ensure(folder, estimateRequiredBytes("editPdf", files.map((f) => f.sizeBytes))))) return;

    await job.run(
      (id) => editPdfOverlays(id, outputPath, buildGroups(refs), toExportDocument(doc)),
      { tool: "editPdf", label: `Edit PDF · ${doc.objects.length} object${doc.objects.length === 1 ? "" : "s"}` },
    );
  };

  const canStart = refs.length > 0 && !!folder && doc.objects.length > 0 && !job.isBusy && inTauri;

  return (
    <ToolPage tool={tool}>
      <ToolSection label="Documents">
        <WorkspaceFilePicker selectable={false} onBeforeRemove={onBeforeRemove} />
        {files.length > 0 && (
          <div className="mt">
            <LargeFileWarning files={files} />
          </div>
        )}
      </ToolSection>

      {!inTauri && (
        <Alert variant="warning">Open the desktop app to edit pages and save a new PDF.</Alert>
      )}

      {editCanvas === "edit" && refs.length > 0 && (
        <ToolSection label="Edit" sublabel="Existing page content stays as-is. Draw on top, then save a new file.">
          <PdfEditorCanvas
            sourcePath={current.path}
            sourcePage={current.page}
            pageIndex={viewIndex}
            pageCount={refs.length}
            session={session}
            onPageChange={setPageIndex}
          />
        </ToolSection>
      )}

      {editCanvas === "no-pages" && (
        <ToolSection label="Edit">
          <Alert variant="warning">This PDF is loaded but has no editable pages.</Alert>
        </ToolSection>
      )}

      <ToolSection label="Output">
        <div className="col">
          <Input label="File name" value={name} onChange={(e) => setName(e.target.value)} placeholder="edited.pdf" />
          <OutputFolderPicker value={folder} onChange={setFolder} />
        </div>
      </ToolSection>

      <div className="row">
        <Button variant="primary" size="lg" onClick={start} disabled={!canStart} loading={job.isBusy} leftIcon={<Icon name="fileText" size={18} />}>
          Save PDF
        </Button>
      </div>

      <JobStatus job={job} />
      {disk.modal}
      <Modal
        open={discardPrompt !== null}
        onClose={() => closeDiscard(false)}
        title="Discard edits on this PDF?"
        footer={
          <>
            <Button variant="secondary" onClick={() => closeDiscard(false)}>
              Cancel
            </Button>
            <Button variant="danger" onClick={() => closeDiscard(true)}>
              Remove PDF
            </Button>
          </>
        }
      >
        {discardPrompt?.historyOnly ? (
          <p>Removing this PDF discards undo history for edits on its pages.</p>
        ) : (
          <p>
            Removing this PDF discards {discardPrompt?.count} edit
            {discardPrompt?.count === 1 ? "" : "s"} on its pages.
          </p>
        )}
      </Modal>
    </ToolPage>
  );
}
