/**
 * Drives a single operation through its lifecycle and mirrors the backend's
 * `job:update` events into React state. Generates the job id, subscribes for
 * progress, records the outcome in the recent-jobs store, and exposes cancel.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { cancelJob, newJobId, onJobUpdate } from "@/lib/tauriCommands";
import {
  toAppError,
  type AppError,
  type JobResult,
  type JobState,
  type JobUpdate,
  type ToolId,
} from "@/lib/types";
import { useJobsStore } from "@/state/jobsStore";

export interface RunMeta {
  tool: ToolId;
  /** Display label for the recent-jobs list, e.g. "Merge 4 files". */
  label: string;
  /** Original total input size, for the "X → Y" result readout. */
  inputBytes?: number;
}

export interface JobController {
  state: JobState;
  update: JobUpdate | null;
  result: JobResult | null;
  error: AppError | null;
  meta: RunMeta | null;
  isBusy: boolean;
  run: (fn: (jobId: string) => Promise<JobResult>, meta: RunMeta) => Promise<void>;
  cancel: () => void;
  reset: () => void;
}

export function useJob(): JobController {
  const [state, setState] = useState<JobState>("idle");
  const [update, setUpdate] = useState<JobUpdate | null>(null);
  const [result, setResult] = useState<JobResult | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [meta, setMeta] = useState<RunMeta | null>(null);
  const jobIdRef = useRef<string | null>(null);
  const mountedRef = useRef(true);
  const addJob = useJobsStore((s) => s.addJob);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const reset = useCallback(() => {
    setState("idle");
    setUpdate(null);
    setResult(null);
    setError(null);
    jobIdRef.current = null;
  }, []);

  const run = useCallback(
    async (fn: (jobId: string) => Promise<JobResult>, meta: RunMeta) => {
      const jobId = newJobId();
      jobIdRef.current = jobId;
      setResult(null);
      setError(null);
      setUpdate(null);
      setMeta(meta);
      setState("preparing");

      let unlisten: (() => void) | undefined;
      try {
        // Register before invoking so no early progress events are missed.
        unlisten = await onJobUpdate((u) => {
          if (!mountedRef.current) return;
          setUpdate(u);
          // Don't let a late "completed/failed" event override our final state.
          if (u.state === "preparing" || u.state === "running") setState(u.state);
        }, jobId);

        const res = await fn(jobId);
        if (!mountedRef.current) return;
        setResult(res);
        setState("completed");
        addJob({
          id: jobId,
          tool: meta.tool,
          label: meta.label,
          status: "completed",
          finishedAt: Date.now(),
          outputPaths: res.outputPaths,
        });
      } catch (e) {
        const appErr = toAppError(e);
        if (!mountedRef.current) return;
        if (appErr.code === "CANCELLED") {
          setState("cancelled");
          addJob({
            id: jobId,
            tool: meta.tool,
            label: meta.label,
            status: "cancelled",
            finishedAt: Date.now(),
            outputPaths: [],
          });
        } else {
          setError(appErr);
          setState("failed");
          addJob({
            id: jobId,
            tool: meta.tool,
            label: meta.label,
            status: "failed",
            finishedAt: Date.now(),
            outputPaths: [],
            error: appErr.title,
          });
        }
      } finally {
        unlisten?.();
      }
    },
    [addJob],
  );

  const cancel = useCallback(() => {
    const id = jobIdRef.current;
    if (!id) return;
    // Optimistic: backend will reject the in-flight command with CANCELLED.
    void cancelJob(id).catch(() => {});
  }, []);

  const isBusy = state === "preparing" || state === "running";
  return { state, update, result, error, meta, isBusy, run, cancel, reset };
}
