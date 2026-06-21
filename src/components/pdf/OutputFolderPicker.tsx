import { Icon } from "@/components/ui/Icon";
import { Button } from "@/components/ui/Button";
import { pickOutputFolder } from "@/lib/tauriCommands";
import { toAppError } from "@/lib/types";
import { useToast } from "@/components/ui/Toast";
import { useSettingsStore } from "@/state/settingsStore";

/** Choose an output folder; remembers the last one in settings. */
export function OutputFolderPicker({
  value,
  onChange,
}: {
  value: string | null;
  onChange: (folder: string) => void;
}) {
  const { toast } = useToast();
  const setLastOutputFolder = useSettingsStore((s) => s.setLastOutputFolder);

  const pick = async () => {
    try {
      const folder = await pickOutputFolder();
      if (folder) {
        onChange(folder);
        setLastOutputFolder(folder);
      }
    } catch (e) {
      toast({ title: "Could not open folder picker", description: toAppError(e).message, variant: "error" });
    }
  };

  return (
    <div className="field">
      <span className="field__label">Output folder</span>
      <div className="row">
        <div
          className="input grow truncate"
          style={{ display: "flex", alignItems: "center", gap: 8, cursor: "default" }}
          title={value ?? undefined}
        >
          <Icon name="folder" size={16} className="subtle" />
          <span className={value ? "truncate" : "subtle truncate"}>
            {value ?? "No folder selected"}
          </span>
        </div>
        <Button variant="secondary" onClick={pick} leftIcon={<Icon name="folderOpen" size={16} />}>
          Browse
        </Button>
      </div>
    </div>
  );
}
