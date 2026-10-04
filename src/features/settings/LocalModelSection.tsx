import { useEffect, useRef, useState } from "react";
import { Badge } from "@/components/ui/Badge";
import { Button } from "@/components/ui/Button";
import { Card } from "@/components/ui/Card";
import { Icon } from "@/components/ui/Icon";
import { Modal } from "@/components/ui/Modal";
import { useToast } from "@/components/ui/Toast";
import { formatBytes } from "@/lib/formatBytes";
import {
  aiImportModel,
  aiListModels,
  aiLoadModel,
  aiPickModelFile,
  aiPreviewModel,
  aiRemoveModel,
  aiUnloadModel,
} from "@/lib/tauriCommands";
import { toAppError, type ModelPreview, type ModelSetupSnapshot } from "@/lib/types";
import { modelSetupView } from "./modelSetupView";

const EMPTY_SNAPSHOT: ModelSetupSnapshot = {
  ready: [],
  backendStatus: "Unloaded",
  health: { ok: false, backendId: "fake" },
  selectedChecksum: null,
  storeRoot: "",
};

export function LocalModelSection() {
  const { toast } = useToast();
  const [snapshot, setSnapshot] = useState<ModelSetupSnapshot>(EMPTY_SNAPSHOT);
  const [preview, setPreview] = useState<ModelPreview | null>(null);
  const [previewPath, setPreviewPath] = useState<string | null>(null);
  const [pendingRemove, setPendingRemove] = useState<{ checksum: string; size: number } | null>(
    null,
  );
  const [busy, setBusy] = useState(false);
  const cancelledRef = useRef(false);

  const view = modelSetupView({
    snapshot,
    preview: preview ?? undefined,
    pendingRemove: pendingRemove ?? undefined,
  });

  const refresh = async () => {
    setSnapshot(await aiListModels());
  };

  useEffect(() => {
    let on = true;
    aiListModels()
      .then((next) => {
        if (on) setSnapshot(next);
      })
      .catch((err) => {
        if (!on) return;
        toast({
          title: "Could not list models",
          description: toAppError(err).message,
          variant: "error",
        });
      });
    return () => {
      on = false;
    };
  }, [toast]);

  const run = async (title: string, work: () => Promise<void>) => {
    if (busy) return;
    cancelledRef.current = false;
    setBusy(true);
    try {
      await work();
    } catch (err) {
      if (cancelledRef.current) return;
      toast({ title, description: toAppError(err).message, variant: "error" });
    } finally {
      setBusy(false);
    }
  };

  const clearPreview = () => {
    setPreview(null);
    setPreviewPath(null);
  };

  const pickPreview = async () => {
    const path = await aiPickModelFile();
    if (!path || cancelledRef.current) return;
    const facts = await aiPreviewModel(path);
    if (cancelledRef.current) return;
    setPreview(facts);
    setPreviewPath(path);
  };

  const onImport = () =>
    run("Could not import model", async () => {
      if (preview && previewPath) {
        try {
          await aiImportModel(previewPath, preview.sha256);
        } catch (err) {
          if (cancelledRef.current) return;
          const appErr = toAppError(err);
          if (
            appErr.code === "AI_MODEL_INVALID" ||
            /checksum/i.test(`${appErr.message} ${appErr.details ?? ""}`)
          ) {
            clearPreview();
          }
          throw err;
        }
        clearPreview();
        await refresh();
        if (cancelledRef.current) return;
        toast({
          title: "Model imported",
          description: "The file was copied into OffPDF app data.",
          variant: "success",
        });
        return;
      }
      await pickPreview();
    });

  const onChooseDifferent = () =>
    run("Could not preview model", async () => {
      clearPreview();
      await pickPreview();
    });

  const onLoad = (checksum: string) =>
    run("Could not load model", async () => {
      await aiLoadModel(checksum);
      await refresh();
    });

  const onUnload = () =>
    run("Could not unload model", async () => {
      await aiUnloadModel();
      await refresh();
    });

  const onCancel = () => {
    cancelledRef.current = true;
    clearPreview();
  };

  const onConfirmRemove = () =>
    run("Could not remove model", async () => {
      if (!pendingRemove) return;
      const result = await aiRemoveModel(pendingRemove.checksum);
      setPendingRemove(null);
      if (preview?.sha256 === result.checksum) {
        clearPreview();
      }
      await refresh();
      toast({
        title: "Model removed",
        description:
          result.recoveredBytes > 0
            ? `Freed ${formatBytes(result.recoveredBytes)}.`
            : "Nothing to clear.",
        variant: "success",
      });
    });

  const facts = view.facts;
  const loaded = snapshot.backendStatus === "Ready";

  return (
    <Card padded>
      <div className="setting-row">
        <div>
          <div className="setting-row__label">Local model</div>
          <div className="setting-row__desc">
            Optional. Import a file from this computer. Nothing is downloaded and PDF tools stay
            available with no model installed.
          </div>
        </div>
        <Badge variant={loaded ? "success" : "neutral"}>{snapshot.backendStatus}</Badge>
      </div>

      {view.showFacts && (
        <dl className="model-facts">
          <div className="model-facts__row">
            <dt>Size</dt>
            <dd>{facts ? formatBytes(facts.size) : "—"}</dd>
          </div>
          <div className="model-facts__row">
            <dt>License</dt>
            <dd>{facts?.license ?? "—"}</dd>
          </div>
          <div className="model-facts__row">
            <dt>Location</dt>
            <dd className="mono">{facts?.location ?? "—"}</dd>
          </div>
          <div className="model-facts__row">
            <dt>Compatibility</dt>
            <dd>
              {facts?.compatibility ?? "Hardware check is not available in this version."}
            </dd>
          </div>
        </dl>
      )}

      {snapshot.ready.length > 0 && (
        <ul className="model-ready-list">
          {snapshot.ready.map((model) => {
            const isSelected =
              loaded && snapshot.selectedChecksum === model.checksum;
            return (
              <li key={model.checksum} className="model-ready">
                <div>
                  <div className="setting-row__label">
                    {model.checksum.slice(0, 8)}
                    <span className="model-ready__size"> {formatBytes(model.size)}</span>
                  </div>
                  <div className="setting-row__desc mono">{model.checksum}</div>
                </div>
                <div className="row gap-sm">
                  {isSelected ? (
                    <Button variant="secondary" onClick={onUnload} disabled={busy}>
                      Unload
                    </Button>
                  ) : (
                    <Button variant="secondary" onClick={() => onLoad(model.checksum)} disabled={busy}>
                      Load
                    </Button>
                  )}
                  <Button
                    variant="danger"
                    onClick={() => setPendingRemove({ checksum: model.checksum, size: model.size })}
                    disabled={busy}
                    leftIcon={<Icon name="trash" size={16} />}
                  >
                    Remove
                  </Button>
                </div>
              </li>
            );
          })}
        </ul>
      )}

      <div className="row gap-sm model-actions">
        <Button variant="primary" onClick={onImport} loading={busy} leftIcon={<Icon name="upload" size={16} />}>
          {view.importCta}
        </Button>
        {preview && (
          <Button variant="secondary" onClick={onChooseDifferent} disabled={busy}>
            Choose a different file
          </Button>
        )}
        <Button variant="secondary" onClick={onCancel} leftIcon={<Icon name="stop" size={16} />}>
          Cancel
        </Button>
      </div>

      <Modal
        open={pendingRemove !== null}
        onClose={() => setPendingRemove(null)}
        title="Remove imported model?"
        footer={
          <>
            <Button variant="secondary" onClick={() => setPendingRemove(null)} disabled={busy}>
              Keep model
            </Button>
            <Button variant="danger" onClick={onConfirmRemove} loading={busy}>
              Remove
            </Button>
          </>
        }
      >
        <p>{view.confirmRemove}</p>
      </Modal>
    </Card>
  );
}
