import { Progress } from "@/components/ui/Progress";
import { Button } from "@/components/ui/Button";
import { Alert } from "@/components/ui/Alert";
import { Details } from "@/components/ui/Details";
import { Icon } from "@/components/ui/Icon";
import { Card } from "@/components/ui/Card";
import { useToast } from "@/components/ui/Toast";
import { useEffect, useState } from "react";
import { getFileInfo, openPath } from "@/lib/tauriCommands";
import { toAppError } from "@/lib/types";
import { basename, dirname, formatBytes } from "@/lib/formatBytes";
import { useWorkspace } from "@/state/workspaceStore";
import type { JobController } from "./useJob";

/** Fetches the total size of the produced files and renders a size readout. */
function SizeReadout({ outputs, inputBytes }: { outputs: string[]; inputBytes?: number }) {
  const [bytes, setBytes] = useState<number | null>(null);
  useEffect(() => {
    let on = true;
    Promise.all(outputs.map((p) => getFileInfo(p).then((i) => i.sizeBytes).catch(() => 0)))
      .then((sizes) => on && setBytes(sizes.reduce((a, b) => a + b, 0)))
      .catch(() => {});
    return () => {
      on = false;
    };
  }, [outputs.join("|")]);

  if (bytes == null) return null;
  const pct = inputBytes && inputBytes > 0 ? Math.round((1 - bytes / inputBytes) * 100) : null;
  return (
    <div className="muted" style={{ textAlign: "center", fontSize: 12.5, marginTop: 6 }}>
      {inputBytes && inputBytes > 0 ? (
        <>
          {formatBytes(inputBytes)} → <b style={{ color: "var(--text)" }}>{formatBytes(bytes)}</b>
          {pct !== null && pct > 0 ? ` · ${pct}% smaller` : ""}
        </>
      ) : (
        <>Output size: {formatBytes(bytes)}</>
      )}
    </div>
  );
}

/** Renders the right panel for the current job state: progress / result / error. */
export function JobStatus({ job }: { job: JobController }) {
  const { toast } = useToast();
  const addPaths = useWorkspace((s) => s.addPaths);

  const continueEditing = async (path: string) => {
    const r = await addPaths([path]);
    if (r.added > 0) {
      toast({ title: "Added to your workspace", description: "Pick any tool to keep editing it.", variant: "success" });
    } else {
      toast({ title: "Could not load the result", variant: "error" });
    }
  };

  const open = async (path: string, what: string) => {
    try {
      await openPath(path);
    } catch (e) {
      toast({ title: `Could not open ${what}`, description: toAppError(e).message, variant: "error" });
    }
  };

  if (job.isBusy) {
    const step = job.update?.step ?? (job.state === "preparing" ? "Preparing…" : "Working…");
    return (
      <Card padded>
        <div className="job-panel">
          <div className="spread">
            <div className="job-panel__step row gap-sm">
              <Icon name="optimize" size={16} className="muted" />
              {step}
            </div>
            <Button variant="danger" size="sm" onClick={job.cancel} leftIcon={<Icon name="stop" size={14} />}>
              Cancel
            </Button>
          </div>
          <Progress value={job.update?.percent ?? undefined} />
          {job.update?.message && <div className="job-log selectable">{job.update.message}</div>}
        </div>
      </Card>
    );
  }

  if (job.state === "completed" && job.result) {
    const outputs = job.result.outputPaths;
    const folder = outputs.length > 0 ? dirname(outputs[0]) : "";
    return (
      <Card padded>
        <div className="result-icon">
          <Icon name="check" size={26} />
        </div>
        <h3 style={{ textAlign: "center" }}>Done</h3>
        <p className="muted" style={{ textAlign: "center" }}>
          {outputs.length === 1
            ? "Your file is ready."
            : `${outputs.length} files are ready.`}
        </p>
        <SizeReadout outputs={outputs} inputBytes={job.meta?.inputBytes} />

        <div className="result-files">
          {outputs.slice(0, 8).map((p) => (
            <div className="file-row" key={p}>
              <div className="file-row__icon">
                <Icon name="fileText" size={18} />
              </div>
              <div className="grow">
                <div className="file-row__name truncate" title={p}>
                  {basename(p)}
                </div>
                <div className="file-row__meta truncate">{p}</div>
              </div>
              <Button variant="ghost" size="sm" onClick={() => open(p, "file")} title="Open file">
                <Icon name="external" size={16} />
              </Button>
            </div>
          ))}
          {outputs.length > 8 && (
            <div className="subtle" style={{ paddingLeft: 4 }}>
              +{outputs.length - 8} more in the output folder
            </div>
          )}
        </div>

        <div className="row mt" style={{ justifyContent: "center" }}>
          {outputs.length === 1 && (
            <Button variant="primary" onClick={() => open(outputs[0], "file")} leftIcon={<Icon name="external" size={16} />}>
              Open output file
            </Button>
          )}
          {folder && (
            <Button variant="secondary" onClick={() => open(folder, "folder")} leftIcon={<Icon name="folderOpen" size={16} />}>
              Open containing folder
            </Button>
          )}
          {outputs.length === 1 && (
            <Button variant="secondary" onClick={() => continueEditing(outputs[0])} leftIcon={<Icon name="arrowRight" size={16} />}>
              Continue editing
            </Button>
          )}
          <Button variant="ghost" onClick={job.reset}>
            Run again
          </Button>
        </div>
      </Card>
    );
  }

  if (job.state === "failed" && job.error) {
    return (
      <Card padded>
        <Alert variant="danger" title={job.error.title}>
          {job.error.message}
          {job.error.suggestion && (
            <div className="mt-sm" style={{ fontWeight: 600 }}>
              {job.error.suggestion}
            </div>
          )}
        </Alert>
        {job.error.details && (
          <div className="mt">
            <Details>{job.error.details}</Details>
          </div>
        )}
        <div className="row mt">
          <Button variant="secondary" onClick={job.reset}>
            Dismiss
          </Button>
        </div>
      </Card>
    );
  }

  if (job.state === "cancelled") {
    return (
      <Card padded>
        <Alert variant="warning" title="Operation cancelled">
          The operation was cancelled before it finished. No output file was kept.
        </Alert>
        <div className="row mt">
          <Button variant="secondary" onClick={job.reset}>
            Dismiss
          </Button>
        </div>
      </Card>
    );
  }

  return null;
}
