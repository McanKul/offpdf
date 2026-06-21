import { useEffect, useState } from "react";
import { ToolPage, ToolSection } from "@/components/tools/ToolPage";
import { Button } from "@/components/ui/Button";
import { Input } from "@/components/ui/Input";
import { Icon } from "@/components/ui/Icon";
import { Alert } from "@/components/ui/Alert";
import { useToast } from "@/components/ui/Toast";
import {
  WorkspaceFilePicker,
  OutputFolderPicker,
  LargeFileWarning,
  useCombinedDoc,
  buildGroups,
} from "@/components/pdf";
import { useJob, JobStatus, useDiskGuard } from "@/components/jobs";
import { protectPdf } from "@/lib/tauriCommands";
import { getTool } from "@/lib/tools";
import { estimateRequiredBytes, validateOutputName, joinPath } from "@/lib/validation";
import { stripExt } from "@/lib/formatBytes";
import { useSettingsStore } from "@/state/settingsStore";
import { useWorkspace } from "@/state/workspaceStore";

const tool = getTool("protect");

export function ProtectPage() {
  const files = useWorkspace((s) => s.files);
  const refs = useCombinedDoc();
  const job = useJob();
  const disk = useDiskGuard();
  const { toast } = useToast();

  const lastFolder = useSettingsStore((s) => s.lastOutputFolder);
  const [folder, setFolder] = useState<string | null>(lastFolder);
  const [name, setName] = useState("protected.pdf");
  const [pw, setPw] = useState("");
  const [pw2, setPw2] = useState("");

  const first = files[0];
  useEffect(() => {
    if (first) setName(`${stripExt(first.name)}-protected.pdf`);
  }, [first?.path]);

  const start = async () => {
    if (refs.length === 0) return toast({ title: "Add a PDF first", variant: "error" });
    if (!folder) return toast({ title: "Choose an output folder", variant: "error" });
    if (pw.length === 0) return toast({ title: "Enter a password", variant: "error" });
    if (pw !== pw2) return toast({ title: "Passwords don't match", variant: "error" });
    const nameRes = validateOutputName(name);
    if (!nameRes.ok) return toast({ title: "Invalid file name", description: nameRes.error, variant: "error" });
    const outputPath = joinPath(folder, nameRes.value);
    if (!(await disk.ensure(folder, estimateRequiredBytes("optimize", files.map((f) => f.sizeBytes))))) return;

    await job.run((id) => protectPdf(id, outputPath, buildGroups(refs), pw, ""), {
      tool: "protect",
      label: `Protect ${first?.name ?? "PDF"}`,
    });
  };

  const canStart = refs.length > 0 && !!folder && pw.length > 0 && pw === pw2 && !job.isBusy;

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

      <ToolSection label="Password">
        <div className="col">
          <Input
            label="Password"
            type="password"
            value={pw}
            onChange={(e) => setPw(e.target.value)}
            placeholder="Set a password to open the PDF"
          />
          <Input
            label="Confirm password"
            type="password"
            value={pw2}
            onChange={(e) => setPw2(e.target.value)}
            error={pw2.length > 0 && pw !== pw2 ? "Passwords don't match." : null}
          />
          <Alert variant="warning" title="Keep your password safe">
            The PDF is encrypted with AES-256. If you forget the password, the file cannot be opened —
            there is no recovery.
          </Alert>
        </div>
      </ToolSection>

      <ToolSection label="Output">
        <div className="col">
          <Input label="File name" value={name} onChange={(e) => setName(e.target.value)} placeholder="protected.pdf" />
          <OutputFolderPicker value={folder} onChange={setFolder} />
        </div>
      </ToolSection>

      <div className="row">
        <Button variant="primary" size="lg" onClick={start} disabled={!canStart} loading={job.isBusy} leftIcon={<Icon name="lock" size={18} />}>
          Protect PDF
        </Button>
      </div>

      <JobStatus job={job} />
      {disk.modal}
    </ToolPage>
  );
}
