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
import { editPdfOverlays, listPdfFormFields, listPdfLinks } from "@/lib/tauriCommands";
import { getTool } from "@/lib/tools";
import type { FormField, FormValue } from "@/lib/editor";
import { estimateRequiredBytes, validateOutputName, joinPath } from "@/lib/validation";
import { stripExt } from "@/lib/formatBytes";
import { useSettingsStore } from "@/state/settingsStore";
import { useWorkspace } from "@/state/workspaceStore";
import {
  clampPageIndex,
  emptyObjectsBlockSave,
  incompleteSourcePaths,
  makeLinkObject,
  planKeyRebind,
  rebindNeedsConfirm,
  resolveViewPageIndex,
  samePageKeys,
  shouldShowEditCanvas,
  toExportDocument,
} from "@/lib/editor";
import { isTauriRuntime } from "@/lib/tauriEnv";
import { toAppError, type AppError } from "@/lib/types";
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
  const [flattenAnnotations, setFlattenAnnotations] = useState(false);
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
  const hydratedUids = useRef(new Set<string>());
  const hydrateErrors = useRef(new Map<string, AppError>());
  const hydratedLinkUids = useRef(new Set<string>());
  const [hydrateReady, setHydrateReady] = useState(
    () => files.every((f) => (f.pageCount ?? 0) === 0),
  );
  const [formFields, setFormFields] = useState<FormField[]>([]);
  const [formValues, setFormValues] = useState<Record<string, string>>({});
  const [formError, setFormError] = useState<string | null>(null);
  const [flattenForm, setFlattenForm] = useState(false);

  useEffect(() => {
    const live = new Set(files.map((f) => f.uid));
    for (const uid of [...hydratedUids.current]) {
      if (!live.has(uid)) {
        hydratedUids.current.delete(uid);
        hydrateErrors.current.delete(uid);
        hydratedLinkUids.current.delete(uid);
      }
    }
    const pending = files.filter((f) => (f.pageCount ?? 0) > 0 && !hydratedUids.current.has(f.uid));
    if (pending.length === 0) {
      setHydrateReady(true);
      return;
    }
    setHydrateReady(false);
    let cancelled = false;
    void (async () => {
      const mapped: import("@/lib/editor").EditObject[] = [];
      const finished: string[] = [];
      for (const file of pending) {
        try {
          const listed = await listPdfLinks(file.path);
          if (cancelled) return;
          for (const link of listed) {
            const pageIndex = refs.findIndex((r) => r.key === `${file.uid}#${link.pageIndex + 1}`);
            if (pageIndex < 0) continue;
            let action = link.action;
            if (action.type === "goto") {
              const destPage = action.destPageIndex;
              const dest = refs.findIndex((r) => r.key === `${file.uid}#${destPage + 1}`);
              if (dest < 0) continue;
              action = { type: "goto", destPageIndex: dest };
            }
            const id =
              typeof crypto !== "undefined" && "randomUUID" in crypto
                ? crypto.randomUUID()
                : `link-${file.uid}-${link.pageIndex}-${mapped.length}`;
            mapped.push(makeLinkObject(id, pageIndex, link.rect, action));
          }
          if (listed.length > 0) {
            hydratedLinkUids.current.add(file.uid);
          } else {
            hydratedLinkUids.current.delete(file.uid);
          }
          hydrateErrors.current.delete(file.uid);
          finished.push(file.uid);
        } catch (e) {
          if (cancelled) return;
          const err = toAppError(e);
          hydrateErrors.current.set(file.uid, err);
          toast({ title: err.title, description: err.message, variant: "error" });
          finished.push(file.uid);
        }
      }
      if (cancelled) return;
      if (mapped.length > 0) session.hydrateObjects(mapped);
      for (const uid of finished) hydratedUids.current.add(uid);
      setHydrateReady(pending.every((f) => hydratedUids.current.has(f.uid)));
    })();
    return () => {
      cancelled = true;
    };
  }, [files, refs, session.hydrateObjects]);

  const first = files[0];
  useEffect(() => {
    if (first) setName(`${stripExt(first.name)}-edited.pdf`);
  }, [first?.path]);

  useEffect(() => {
    if (!inTauri || files.length === 0) {
      setFormFields([]);
      setFormValues({});
      setFormError(null);
      return;
    }
    let active = true;
    const primary = files[0].path;
    const extras = files.slice(1).map((f) => f.path);
    (async () => {
      try {
        for (const extra of extras) {
          try {
            const extraFields = await listPdfFormFields(extra);
            if (!active) return;
            if (extraFields.length > 0) {
              setFormFields([]);
              setFormValues({});
              setFormError(
                "Only the first PDF's form can be filled. Remove extra files that have form fields.",
              );
              return;
            }
          } catch {
            if (!active) return;
            setFormFields([]);
            setFormValues({});
            setFormError(
              "Only the first PDF's form can be filled. An extra file has a form OffPDF cannot merge.",
            );
            return;
          }
        }
        const listed = await listPdfFormFields(primary);
        if (!active) return;
        setFormError(null);
        setFormFields(listed);
        setFormValues((prev) => {
          const next: Record<string, string> = {};
          for (const f of listed) {
            next[f.name] = prev[f.name] ?? f.value ?? "";
          }
          return next;
        });
      } catch (e) {
        if (!active) return;
        const err = e as { title?: string; message?: string; code?: string };
        setFormFields([]);
        setFormValues({});
        setFormError(err.message ?? "Could not read form fields.");
      }
    })();
    return () => {
      active = false;
    };
  }, [files, inTauri]);

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

  const formPayload: FormValue[] = formFields.map((f) => ({
    name: f.name,
    value: formValues[f.name] ?? f.value ?? "",
  }));
  const formDirty = formFields.some((f) => (formValues[f.name] ?? "") !== (f.value ?? ""));
  const canSaveEdits = doc.objects.length > 0 || formDirty || (flattenForm && formFields.length > 0);

  const start = async () => {
    if (refs.length === 0) return toast({ title: "Add a PDF first", variant: "error" });
    if (!hydrateReady) return toast({ title: "Still reading links from the PDF", variant: "error" });
    if (formError) return toast({ title: "Cannot fill this form", description: formError, variant: "error" });
    if (
      emptyObjectsBlockSave({
        objectCount: doc.objects.length,
        hadHydratedLinks: hydratedLinkUids.current.size > 0,
      }) &&
      !canSaveEdits
    ) {
      return toast({ title: "Add something to the page first", variant: "error" });
    }
    if (!folder) return toast({ title: "Choose an output folder", variant: "error" });
    const nameRes = validateOutputName(name);
    if (!nameRes.ok) return toast({ title: "Invalid file name", description: nameRes.error, variant: "error" });
    const outputPath = joinPath(folder, nameRes.value);
    if (files.some((f) => f.path === outputPath)) {
      return toast({ title: "Choose a new file name", description: "The original file is never overwritten.", variant: "error" });
    }
    if (!(await disk.ensure(folder, estimateRequiredBytes("editPdf", files.map((f) => f.sizeBytes))))) return;

    const failedLinkErr = sessionLinkErrorOnFailedFile(doc.objects, refs, hydrateErrors.current);
    if (failedLinkErr) {
      return toast({ title: failedLinkErr.title, description: failedLinkErr.message, variant: "error" });
    }
    const incompletePaths = incompleteSourcePaths(
      files,
      new Set(hydrateErrors.current.keys()),
    );
    const values = formDirty || flattenForm ? formPayload : [];
    await job.run(
      (id) =>
        editPdfOverlays(
          id,
          outputPath,
          buildGroups(refs),
          toExportDocument(doc),
          incompletePaths,
          values,
          flattenForm,
          flattenAnnotations,
        ),
      {
        tool: "editPdf",
        label: `Edit PDF · ${doc.objects.length} object${doc.objects.length === 1 ? "" : "s"}`,
      },
    );
  };

  const canStart = !job.isBusy && inTauri && hydrateReady;

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

      <ToolSection label="Output">
        <div className="col">
          <Input label="File name" value={name} onChange={(e) => setName(e.target.value)} placeholder="edited.pdf" />
          <OutputFolderPicker value={folder} onChange={setFolder} />
          {formFields.length > 0 && (
            <label className="row" style={{ gap: 8, alignItems: "center", fontSize: 13.5 }}>
              <input
                type="checkbox"
                checked={flattenForm}
                onChange={(e) => {
                  setFlattenForm(e.target.checked);
                  if (!e.target.checked) setFlattenAnnotations(false);
                }}
              />
              Flatten form fields (widgets become page content)
            </label>
          )}
          <label className="field__label" style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <input
              type="checkbox"
              checked={flattenAnnotations}
              onChange={(e) => {
                setFlattenAnnotations(e.target.checked);
                if (e.target.checked && formFields.length > 0) setFlattenForm(true);
              }}
            />
            Flatten annotations{formFields.length > 0 ? " (includes form fields)" : ""}
          </label>
          <div className="row">
            <Button variant="primary" size="lg" onClick={start} disabled={!canStart} loading={job.isBusy} leftIcon={<Icon name="fileText" size={18} />}>
              Save PDF
            </Button>
          </div>
        </div>
      </ToolSection>

      <JobStatus job={job} />

      {editCanvas === "edit" && refs.length > 0 && (
        <ToolSection label="Edit">
          {formError && (
            <Alert variant="warning">{formError}</Alert>
          )}
          <PdfEditorCanvas
            sourcePath={current.path}
            sourcePage={current.page}
            pageIndex={viewIndex}
            pageCount={refs.length}
            session={session}
            onPageChange={setPageIndex}
            formFields={current.path === first?.path ? formFields : []}
            formValues={formValues}
            onFormChange={(name, value) => setFormValues((prev) => ({ ...prev, [name]: value }))}
          />
        </ToolSection>
      )}

      {editCanvas === "no-pages" && (
        <ToolSection label="Edit">
          <Alert variant="warning">This PDF is loaded but has no editable pages.</Alert>
        </ToolSection>
      )}

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

function fileUidFromPageKey(key: string): string {
  const hash = key.lastIndexOf("#");
  return hash >= 0 ? key.slice(0, hash) : key;
}

function sessionLinkErrorOnFailedFile(
  objects: { kind: string; pageIndex: number }[],
  refs: { key: string }[],
  errors: Map<string, AppError>,
): AppError | undefined {
  for (const o of objects) {
    if (o.kind !== "link") continue;
    const ref = refs[o.pageIndex];
    if (!ref) continue;
    const err = errors.get(fileUidFromPageKey(ref.key));
    if (err) return err;
  }
  return undefined;
}
