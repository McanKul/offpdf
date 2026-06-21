import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { Card } from "@/components/ui/Card";
import { Button } from "@/components/ui/Button";
import { Badge } from "@/components/ui/Badge";
import { Select } from "@/components/ui/Select";
import { Icon } from "@/components/ui/Icon";
import { Alert } from "@/components/ui/Alert";
import { useToast } from "@/components/ui/Toast";
import { useSettingsStore, type Theme } from "@/state/settingsStore";
import { clearTempFiles, getTempDir, openPath } from "@/lib/tauriCommands";
import { formatBytes } from "@/lib/formatBytes";
import { toAppError } from "@/lib/types";

export function SettingsPage() {
  const theme = useSettingsStore((s) => s.theme);
  const setTheme = useSettingsStore((s) => s.setTheme);
  const { toast } = useToast();

  const [version, setVersion] = useState<string>("—");
  const [tempDir, setTempDir] = useState<string>("");
  const [clearing, setClearing] = useState(false);

  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion("0.1.0"));
    getTempDir().then(setTempDir).catch(() => setTempDir(""));
  }, []);

  const onClearTemp = async () => {
    setClearing(true);
    try {
      const freed = await clearTempFiles();
      toast({
        title: "Temp files cleared",
        description: freed > 0 ? `Freed ${formatBytes(freed)}.` : "Nothing to clear.",
        variant: "success",
      });
    } catch (e) {
      toast({ title: "Could not clear temp files", description: toAppError(e).message, variant: "error" });
    } finally {
      setClearing(false);
    }
  };

  const onOpenTemp = async () => {
    try {
      const dir = tempDir || (await getTempDir());
      await openPath(dir);
    } catch (e) {
      toast({ title: "Could not open folder", description: toAppError(e).message, variant: "error" });
    }
  };

  return (
    <div className="col gap-lg">
      <div className="page-header">
        <div className="page-header__top">
          <div className="page-header__icon">
            <Icon name="settings" size={22} />
          </div>
          <h1 className="page-header__title">Settings</h1>
        </div>
        <p className="page-header__desc">
          OffPDF is offline-only. There is nothing to log in to and nothing to sync.
        </p>
      </div>

      <Card padded>
        <div className="setting-row">
          <div>
            <div className="setting-row__label">Offline mode</div>
            <div className="setting-row__desc">
              Always on for this version. The app never makes a network request.
            </div>
          </div>
          <Badge variant="success">
            <Icon name="lock" size={12} /> Always on
          </Badge>
        </div>

        <div className="setting-row">
          <div>
            <div className="setting-row__label">Appearance</div>
            <div className="setting-row__desc">Choose light, dark, or follow your system.</div>
          </div>
          <div style={{ width: 180 }}>
            <Select
              value={theme}
              onChange={(v) => setTheme(v as Theme)}
              options={[
                { value: "light", label: "Light" },
                { value: "dark", label: "Dark" },
                { value: "system", label: "System" },
              ]}
            />
          </div>
        </div>

        <div className="setting-row">
          <div>
            <div className="setting-row__label">Temporary files</div>
            <div className="setting-row__desc">
              Jobs use a private temp folder that is normally cleaned automatically.
              {tempDir && <span className="mono"> {tempDir}</span>}
            </div>
          </div>
          <div className="row gap-sm">
            <Button variant="secondary" onClick={onOpenTemp} leftIcon={<Icon name="folderOpen" size={16} />}>
              Open folder
            </Button>
            <Button variant="secondary" onClick={onClearTemp} loading={clearing} leftIcon={<Icon name="trash" size={16} />}>
              Clear temp files
            </Button>
          </div>
        </div>

        <div className="setting-row">
          <div>
            <div className="setting-row__label">App version</div>
            <div className="setting-row__desc">OffPDF — local-first PDF utility.</div>
          </div>
          <Badge variant="neutral">v{version}</Badge>
        </div>
      </Card>

      <Alert variant="success" title="Privacy statement" icon="shield">
        OffPDF processes everything on your computer. It does not upload your files, file
        names, paths, metadata, thumbnails, or any analytics. It works without an internet
        connection.
      </Alert>
    </div>
  );
}
