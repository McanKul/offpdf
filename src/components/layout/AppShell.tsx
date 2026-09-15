import { useEffect } from "react";
import { Outlet } from "react-router-dom";
import { Sidebar } from "./Sidebar";
import { useToast } from "@/components/ui/Toast";
import { isTauriRuntime } from "@/lib/tauriEnv";
import { onOpenedPaths, takeOpenedPaths } from "@/lib/tauriCommands";
import { useWorkspace, type AddResult } from "@/state/workspaceStore";

function reportIntake(
  toast: (input: {
    title: string;
    description?: string;
    variant?: "success" | "error" | "info";
  }) => void,
  result: AddResult,
) {
  if (result.notPdf) {
    toast({ title: "Only PDF, image, or Office files are supported", variant: "error" });
  }
  if (result.errors.length) {
    toast({
      title: "Some files could not be added",
      description: result.errors.join(" · "),
      variant: "error",
    });
  }
  if (result.invalid.length) {
    toast({
      title: result.invalid.length === 1 ? "That file is not a valid PDF" : "Some files are not valid PDFs",
      description: result.invalid.join(", "),
      variant: "error",
    });
  }
}

export function AppShell() {
  const { toast } = useToast();

  useEffect(() => {
    if (!isTauriRuntime()) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    let intakeQueue = Promise.resolve();

    const apply = (paths: string[]) => {
      intakeQueue = intakeQueue.then(async () => {
        if (!paths.length || cancelled) return;
        const result = await useWorkspace.getState().addPaths(paths, { dedupe: true });
        if (!cancelled) reportIntake(toast, result);
      });
    };

    void (async () => {
      try {
        const stop = await onOpenedPaths((paths) => {
          apply(paths);
        });
        if (cancelled) {
          stop();
          return;
        }
        unlisten = stop;
        const pending = await takeOpenedPaths();
        apply(pending);
        await intakeQueue;
      } catch {
        // Browser / missing IPC — in-app picker still works.
      }
    })();

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [toast]);

  return (
    <div className="app-shell">
      <Sidebar />
      <div className="main">
        <main className="content">
          <div className="content__inner">
            <Outlet />
          </div>
        </main>
      </div>
    </div>
  );
}
