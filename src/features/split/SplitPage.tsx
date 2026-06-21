import { useState } from "react";
import { ToolPage, ToolSection } from "@/components/tools/ToolPage";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Icon } from "@/components/ui/Icon";
import { Tabs, type TabItem } from "@/components/ui/Tabs";
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
import { splitPdf } from "@/lib/tauriCommands";
import { getTool } from "@/lib/tools";
import { estimateRequiredBytes } from "@/lib/validation";
import type { SplitMode } from "@/lib/types";
import { useSettingsStore } from "@/state/settingsStore";
import { useWorkspace } from "@/state/workspaceStore";

const tool = getTool("split");

type Mode = "everyN" | "ranges";

const MODE_TABS: TabItem<Mode>[] = [
  { id: "everyN", label: "Every N pages" },
  { id: "ranges", label: "By ranges" },
];

/** Parse "1-5, 6-10, 11" into [{start,end}] over a doc of `total` pages. */
function parseRanges(
  text: string,
  total: number,
): { ok: true; ranges: { start: number; end: number }[] } | { ok: false; error: string } {
  const tokens = text.split(/[,\n]/).map((t) => t.trim()).filter(Boolean);
  if (tokens.length === 0) return { ok: false, error: "Enter at least one range, e.g. 1-5, 6-10." };
  const ranges: { start: number; end: number }[] = [];
  for (const tok of tokens) {
    const m = tok.match(/^(\d+)\s*-\s*(\d+)$/) || tok.match(/^(\d+)$/);
    if (!m) return { ok: false, error: `“${tok}” is not a valid range (use 1-5 or 7).` };
    const start = Number(m[1]);
    const end = Number(m[2] ?? m[1]);
    if (start < 1 || end < 1 || start > end) return { ok: false, error: `“${tok}” is not a valid range.` };
    if (end > total) return { ok: false, error: `“${tok}” is beyond the ${total}-page document.` };
    ranges.push({ start, end });
  }
  return { ok: true, ranges };
}

export function SplitPage() {
  const files = useWorkspace((s) => s.files);
  const refs = useCombinedDoc();
  const job = useJob();
  const disk = useDiskGuard();
  const { toast } = useToast();

  const lastFolder = useSettingsStore((s) => s.lastOutputFolder);
  const [folder, setFolder] = useState<string | null>(lastFolder);

  const [mode, setMode] = useState<Mode>("everyN");
  const [n, setN] = useState("10");
  const [rangesText, setRangesText] = useState("");

  const total = refs.length;

  const run = async () => {
    if (total === 0) return toast({ title: "Add a PDF first", variant: "error" });
    if (!folder) return toast({ title: "Choose an output folder", variant: "error" });

    let splitMode: SplitMode;
    if (mode === "everyN") {
      const v = Number(n);
      if (!Number.isInteger(v) || v < 1) return toast({ title: "Enter a whole number ≥ 1 for N", variant: "error" });
      splitMode = { type: "everyN", n: v };
    } else {
      const res = parseRanges(rangesText, total);
      if (!res.ok) return toast({ title: "Check the ranges", description: res.error, variant: "error" });
      splitMode = { type: "ranges", ranges: res.ranges };
    }

    if (!(await disk.ensure(folder, estimateRequiredBytes("split", files.map((f) => f.sizeBytes))))) return;

    await job.run((id) => splitPdf(id, folder, buildPicks(refs), splitMode), {
      tool: "split",
      label: `Split ${total} pages`,
    });
  };

  const everyNCount = mode === "everyN" && Number(n) >= 1 ? Math.ceil(total / Number(n)) : 0;
  const rangesPreview = mode === "ranges" ? parseRanges(rangesText, total) : null;
  const modeReady =
    mode === "everyN" ? Number.isInteger(Number(n)) && Number(n) >= 1 : rangesPreview?.ok === true;

  const canStart = total > 0 && !!folder && modeReady && !job.isBusy;

  return (
    <ToolPage tool={tool}>
      <ToolSection label="Documents" sublabel="All loaded files are combined, then split into separate files.">
        <WorkspaceFilePicker selectable={false} />
        {files.length > 0 && (
          <div className="mt">
            <LargeFileWarning files={files} />
          </div>
        )}
      </ToolSection>

      {total > 0 && (
        <ToolSection label="How to split" sublabel={`The combined document has ${total} pages.`}>
          <Tabs tabs={MODE_TABS} active={mode} onChange={setMode} />
          <div className="mt">
            {mode === "everyN" ? (
              <Input
                label="Pages per file (N)"
                type="number"
                min={1}
                value={n}
                onChange={(e) => setN(e.target.value)}
                hint={everyNCount > 0 ? `Creates ${everyNCount} file${everyNCount === 1 ? "" : "s"}.` : "Each output file will contain at most N pages."}
              />
            ) : (
              <Input
                label="Ranges (one file each)"
                value={rangesText}
                onChange={(e) => setRangesText(e.target.value)}
                placeholder="e.g. 1-5, 6-10, 11-14"
                error={rangesText.trim() && rangesPreview && !rangesPreview.ok ? rangesPreview.error : null}
                hint={
                  rangesPreview?.ok
                    ? `Creates ${rangesPreview.ranges.length} file${rangesPreview.ranges.length === 1 ? "" : "s"}.`
                    : `Comma-separated ranges over 1–${total}; each becomes its own file.`
                }
              />
            )}
          </div>
          <div className="mt">
            <CombinedPreview />
          </div>
        </ToolSection>
      )}

      <ToolSection label="Output">
        <OutputFolderPicker value={folder} onChange={setFolder} />
      </ToolSection>

      <div className="row">
        <Button variant="primary" size="lg" onClick={run} disabled={!canStart} loading={job.isBusy} leftIcon={<Icon name="split" size={18} />}>
          Split PDF
        </Button>
      </div>

      <JobStatus job={job} />
      {disk.modal}
    </ToolPage>
  );
}
