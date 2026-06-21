import { useEffect, useState } from "react";
import { ToolPage, ToolSection } from "@/components/tools/ToolPage";
import { Button } from "@/components/ui/Button";
import { Icon } from "@/components/ui/Icon";
import { Alert } from "@/components/ui/Alert";
import { useToast } from "@/components/ui/Toast";
import { Dropzone, OutputFolderPicker } from "@/components/pdf";
import { useJob, JobStatus, useDiskGuard } from "@/components/jobs";
import { officeToPdfBatch, officeAvailable } from "@/lib/tauriCommands";
import { getTool } from "@/lib/tools";
import { isOfficePath } from "@/lib/fileTypes";
import { basename } from "@/lib/formatBytes";
import { useSettingsStore } from "@/state/settingsStore";

const tool = getTool("officeToPdf");

export function OfficeToPdfPage() {
  const job = useJob();
  const disk = useDiskGuard();
  const { toast } = useToast();
  const lastFolder = useSettingsStore((s) => s.lastOutputFolder);

  const [paths, setPaths] = useState<string[]>([]);
  const [folder, setFolder] = useState<string | null>(lastFolder);
  const [available, setAvailable] = useState(true);

  useEffect(() => {
    let on = true;
    officeAvailable().then((v) => on && setAvailable(v)).catch(() => {});
    return () => {
      on = false;
    };
  }, []);

  const add = (incoming: string[]) => {
    const office = incoming.filter(isOfficePath);
    if (office.length === 0) {
      toast({ title: "Only Office files (Word/Excel/PowerPoint) here", variant: "error" });
      return;
    }
    setPaths((prev) => Array.from(new Set([...prev, ...office])));
  };

  const start = async () => {
    if (paths.length === 0) return toast({ title: "Add Office files first", variant: "error" });
    if (!folder) return toast({ title: "Choose an output folder", variant: "error" });
    if (!(await disk.ensure(folder, 0))) return;
    await job.run((id) => officeToPdfBatch(id, folder, paths), {
      tool: "officeToPdf",
      label: `${paths.length} file(s) → PDF`,
    });
  };

  const canStart = paths.length > 0 && !!folder && available && !job.isBusy;

  return (
    <ToolPage tool={tool}>
      {!available && (
        <Alert variant="warning" title="LibreOffice needed">
          Office conversion uses LibreOffice, which wasn’t found. Install it from libreoffice.org
          (macOS: <span className="mono">brew install --cask libreoffice</span>), then reopen this tool.
        </Alert>
      )}

      <ToolSection label="Office files" sublabel="Word, Excel, PowerPoint, OpenDocument, RTF.">
        <Dropzone multiple onFiles={add} title="Drop Office files here, or click to browse" hint="Converted to PDF locally with LibreOffice." />
        {paths.length > 0 && (
          <div className="file-list mt">
            {paths.map((p, i) => (
              <div className="file-row" key={p}>
                <div className="file-row__icon">
                  <Icon name="fileText" size={18} />
                </div>
                <div className="grow">
                  <div className="file-row__name truncate" title={p}>
                    {basename(p)}
                  </div>
                </div>
                <button className="btn btn--ghost btn--sm" onClick={() => setPaths((prev) => prev.filter((_, idx) => idx !== i))} title="Remove">
                  <Icon name="x" size={16} />
                </button>
              </div>
            ))}
          </div>
        )}
      </ToolSection>

      <ToolSection label="Output">
        <OutputFolderPicker value={folder} onChange={setFolder} />
      </ToolSection>

      <div className="row">
        <Button variant="primary" size="lg" onClick={start} disabled={!canStart} loading={job.isBusy} leftIcon={<Icon name="fileText" size={18} />}>
          Convert to PDF
        </Button>
      </div>

      <JobStatus job={job} />
      {disk.modal}
    </ToolPage>
  );
}
