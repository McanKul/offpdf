import { useEffect, useRef, useState } from "react";
import { ToolPage, ToolSection } from "@/components/tools/ToolPage";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Select } from "@/components/ui/Select";
import { Icon } from "@/components/ui/Icon";
import { Alert } from "@/components/ui/Alert";
import { Spinner } from "@/components/ui/Spinner";
import { useToast } from "@/components/ui/Toast";
import {
  WorkspaceFilePicker,
  OutputFolderPicker,
  LargeFileWarning,
  useCombinedDoc,
  buildGroups,
} from "@/components/pdf";
import { useJob, JobStatus, useDiskGuard } from "@/components/jobs";
// `detectBlankPages` is added by INTEGRATION-utils.md (tauriCommands.ts).
import {
  assemblePdf,
  cancelJob,
  detectBlankPages,
  newJobId,
  onJobUpdate,
  renderThumbnails,
} from "@/lib/tauriCommands";
import { getTool } from "@/lib/tools";
import { toAppError } from "@/lib/types";
import { estimateRequiredBytes, validateOutputName, joinPath } from "@/lib/validation";
import { stripExt } from "@/lib/formatBytes";
import { useSettingsStore } from "@/state/settingsStore";
import { useWorkspace } from "@/state/workspaceStore";

const tool = getTool("blankPages");

type Sensitivity = "strict" | "normal" | "aggressive";

interface BlankHit {
  /** Same key format as `useCombinedDoc` refs: `<path>#<page>`. */
  key: string;
  path: string;
  fileName: string;
  page: number;
  thumb?: string;
}

const THUMB_SIZE = 200;

export function BlankPage() {
  const files = useWorkspace((s) => s.files);
  const refs = useCombinedDoc();
  const job = useJob();
  const disk = useDiskGuard();
  const { toast } = useToast();

  const lastFolder = useSettingsStore((s) => s.lastOutputFolder);
  const [folder, setFolder] = useState<string | null>(lastFolder);
  const [name, setName] = useState("cleaned.pdf");

  const [sensitivity, setSensitivity] = useState<Sensitivity>("normal");
  const [isDetecting, setIsDetecting] = useState(false);
  const [detectStep, setDetectStep] = useState("");
  const [hits, setHits] = useState<BlankHit[] | null>(null);
  /** Keys of detected pages the user decided to KEEP after all. */
  const [spared, setSpared] = useState<Set<string>>(new Set());
  const detectJobRef = useRef<string | null>(null);

  const first = files[0];
  useEffect(() => {
    if (first) setName(`${stripExt(first.name)}-cleaned.pdf`);
  }, [first?.path]);

  // Results are keyed to the loaded files; drop them when the workspace changes.
  const filesSig = files.map((f) => f.path).join("|");
  useEffect(() => {
    setHits(null);
    setSpared(new Set());
  }, [filesSig]);

  const detect = async () => {
    if (files.length === 0) return toast({ title: "Add a PDF first", variant: "error" });
    setIsDetecting(true);
    setHits(null);
    setSpared(new Set());
    setDetectStep("Preparing");
    const jobId = newJobId();
    detectJobRef.current = jobId;
    const unlisten = await onJobUpdate((u) => setDetectStep(u.step), jobId);
    try {
      const found: BlankHit[] = [];
      for (const f of files) {
        const pages = await detectBlankPages(jobId, f.path, sensitivity);
        for (const p of pages) {
          found.push({ key: `${f.path}#${p}`, path: f.path, fileName: f.name, page: p });
        }
      }
      // Thumbnails so the user can confirm what gets removed.
      const byFile = new Map<string, number[]>();
      for (const h of found) {
        byFile.set(h.path, [...(byFile.get(h.path) ?? []), h.page]);
      }
      for (const [path, pages] of byFile) {
        try {
          const thumbs = await renderThumbnails(path, pages, THUMB_SIZE);
          for (const t of thumbs) {
            const hit = found.find((h) => h.path === path && h.page === t.page);
            if (hit) hit.thumb = t.dataUrl;
          }
        } catch {
          /* thumbnails are best effort — page numbers still shown */
        }
      }
      setHits(found);
      if (found.length === 0) {
        toast({ title: "No blank pages found", description: "Try the aggressive sensitivity for noisy scans." });
      }
    } catch (e) {
      const err = toAppError(e);
      if (err.code !== "CANCELLED") {
        toast({ title: err.title, description: err.message, variant: "error" });
      }
    } finally {
      unlisten();
      detectJobRef.current = null;
      setIsDetecting(false);
    }
  };

  const cancelDetect = () => {
    if (detectJobRef.current) void cancelJob(detectJobRef.current);
  };

  const toggleSpare = (key: string) => {
    setSpared((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const removing = (hits ?? []).filter((h) => !spared.has(h.key));

  const removePages = async () => {
    if (!hits || removing.length === 0) return;
    if (!folder) return toast({ title: "Choose an output folder", variant: "error" });
    const nameRes = validateOutputName(name);
    if (!nameRes.ok) return toast({ title: "Invalid file name", description: nameRes.error, variant: "error" });
    const outputPath = joinPath(folder, nameRes.value);

    const removedKeys = new Set(removing.map((h) => h.key));
    const keep = refs.filter((r) => !removedKeys.has(`${r.path}#${r.page}`));
    if (keep.length === 0) {
      return toast({ title: "Cannot remove every page", description: "At least one page must remain.", variant: "error" });
    }
    if (!(await disk.ensure(folder, estimateRequiredBytes("optimize", files.map((f) => f.sizeBytes))))) return;

    // Removal reuses the existing assemble path: keep pages, in order, one PDF.
    await job.run((id) => assemblePdf(id, outputPath, buildGroups(keep)), {
      tool: "blankPages",
      label: `Remove ${removing.length} blank page(s)`,
    });
  };

  const canDetect = files.length > 0 && !isDetecting && !job.isBusy;
  const canRemove = !!hits && removing.length > 0 && !!folder && !isDetecting && !job.isBusy;

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

      <ToolSection label="Detect" sublabel="Pages are scanned locally at a tiny resolution.">
        <div className="col">
          <Select
            label="Sensitivity"
            value={sensitivity}
            onChange={(v) => setSensitivity(v as Sensitivity)}
            options={[
              { value: "strict", label: "Strict — only truly empty pages" },
              { value: "normal", label: "Normal — recommended" },
              { value: "aggressive", label: "Aggressive — also specks & punch holes" },
            ]}
            hint="Uniform scanner gray (an empty sheet scanned as flat noise) is always treated as blank."
          />
          <div className="row">
            <Button
              variant="secondary"
              onClick={detect}
              disabled={!canDetect}
              leftIcon={isDetecting ? undefined : <Icon name="search" size={16} />}
            >
              {isDetecting ? "Scanning…" : "Detect blank pages"}
            </Button>
            {isDetecting && (
              <>
                <Spinner />
                <span className="muted" style={{ fontSize: 12.5 }}>{detectStep}</span>
                <Button size="sm" variant="ghost" onClick={cancelDetect}>
                  Cancel
                </Button>
              </>
            )}
          </div>
        </div>
      </ToolSection>

      {hits && hits.length > 0 && (
        <ToolSection
          label={`Detected ${hits.length} blank page(s)`}
          sublabel="Click a page to keep it instead of removing it."
        >
          <div
            style={{
              display: "grid",
              gridTemplateColumns: "repeat(auto-fill, minmax(110px, 1fr))",
              gap: 10,
            }}
          >
            {hits.map((h) => {
              const kept = spared.has(h.key);
              return (
                <button
                  key={h.key}
                  type="button"
                  onClick={() => toggleSpare(h.key)}
                  title={kept ? "Will be kept — click to remove" : "Will be removed — click to keep"}
                  style={{
                    border: kept ? "2px solid var(--success, #16a34a)" : "2px solid var(--danger, #dc2626)",
                    borderRadius: 8,
                    padding: 4,
                    background: "transparent",
                    cursor: "pointer",
                    opacity: kept ? 1 : 0.85,
                  }}
                >
                  {h.thumb ? (
                    <img src={h.thumb} alt={`Page ${h.page}`} style={{ width: "100%", display: "block", borderRadius: 4 }} />
                  ) : (
                    <div className="muted" style={{ padding: "24px 0", textAlign: "center" }}>
                      <Icon name="file" size={22} />
                    </div>
                  )}
                  <div className="muted" style={{ fontSize: 12, marginTop: 4, textAlign: "center" }}>
                    {files.length > 1 ? `${h.fileName} · ` : ""}p.{h.page} — {kept ? "keep" : "remove"}
                  </div>
                </button>
              );
            })}
          </div>
          <div className="mt">
            <Alert variant="info">
              {removing.length} page(s) will be removed; {refs.length - removing.length} will remain.
            </Alert>
          </div>
        </ToolSection>
      )}

      {hits && hits.length === 0 && (
        <ToolSection label="Result">
          <Alert variant="success" title="No blank pages detected" icon="checkCircle">
            Nothing to remove at the “{sensitivity}” sensitivity.
          </Alert>
        </ToolSection>
      )}

      <ToolSection label="Output">
        <div className="col">
          <Input label="File name" value={name} onChange={(e) => setName(e.target.value)} placeholder="cleaned.pdf" />
          <OutputFolderPicker value={folder} onChange={setFolder} />
        </div>
      </ToolSection>

      <div className="row">
        <Button
          variant="primary"
          size="lg"
          onClick={removePages}
          disabled={!canRemove}
          loading={job.isBusy}
          leftIcon={<Icon name="trash" size={18} />}
        >
          Remove {removing.length > 0 ? `${removing.length} ` : ""}blank page(s)
        </Button>
      </div>

      <JobStatus job={job} />
      {disk.modal}
    </ToolPage>
  );
}
