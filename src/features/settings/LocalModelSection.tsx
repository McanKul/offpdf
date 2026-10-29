import { useEffect, useRef, useState } from "react";
import { Alert } from "@/components/ui/Alert";
import { Badge } from "@/components/ui/Badge";
import { Button } from "@/components/ui/Button";
import { Card } from "@/components/ui/Card";
import { Icon } from "@/components/ui/Icon";
import { Input } from "@/components/ui/Input";
import { Modal } from "@/components/ui/Modal";
import { useToast } from "@/components/ui/Toast";
import { formatBytes } from "@/lib/formatBytes";
import {
  aiCancel,
  aiGenerate,
  aiImportModel,
  aiListModels,
  aiLoadModel,
  aiPickModelFile,
  aiPreviewModel,
  aiRemoveModel,
  aiUnloadModel,
} from "@/lib/tauriCommands";
import { toAppError, type ModelPreview, type ModelSetupSnapshot } from "@/lib/types";
import { licenseLabel, modelSetupView } from "./modelSetupView";

const EMPTY_SNAPSHOT: ModelSetupSnapshot = {
  ready: [],
  backendStatus: "Unloaded",
  health: { ok: false, backendId: "fake" },
  selectedChecksum: null,
  storeRoot: "",
};

type Operation = "idle" | "preview" | "import" | "load" | "unload" | "generate" | "remove";

export function LocalModelSection() {
  const { toast } = useToast();
  const [snapshot, setSnapshot] = useState<ModelSetupSnapshot>(EMPTY_SNAPSHOT);
  const [preview, setPreview] = useState<ModelPreview | null>(null);
  const [previewPath, setPreviewPath] = useState<string | null>(null);
  const [pendingRemove, setPendingRemove] = useState<{ checksum: string; size: number } | null>(
    null,
  );
  const [operation, setOperation] = useState<Operation>("idle");
  const [prompt, setPrompt] = useState("health check");
  const [reply, setReply] = useState("");
  const cancelledRef = useRef(false);

  const busy = operation !== "idle";
  const canCancel = operation === "load" || operation === "generate";
  const fakeBackend = snapshot.health.backendId === "fake";
  const loaded = snapshot.backendStatus === "Ready";

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

  const run = async (title: string, next: Operation, work: () => Promise<void>) => {
    if (busy) return;
    cancelledRef.current = false;
    setOperation(next);
    try {
      await work();
    } catch (err) {
      if (cancelledRef.current) return;
      toast({ title, description: toAppError(err).message, variant: "error" });
    } finally {
      setOperation("idle");
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
    run("Could not import model", "import", async () => {
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
    run("Could not preview model", "preview", async () => {
      clearPreview();
      await pickPreview();
    });

  const onLoad = (checksum: string) =>
    run("Could not load model", "load", async () => {
      setReply("");
      try {
        await aiLoadModel(checksum);
      } finally {
        await refresh();
      }
    });

  const onUnload = () =>
    run("Could not unload model", "unload", async () => {
      setReply("");
      await aiUnloadModel();
      await refresh();
    });

  const onGenerate = () =>
    run("Could not run the test prompt", "generate", async () => {
      setReply("");
      const text = prompt.trim();
      if (!text) return;
      const output = await aiGenerate(text);
      if (!cancelledRef.current) setReply(output);
    });

  const onCancel = () => {
    if (!canCancel) return;
    cancelledRef.current = true;
    void aiCancel().catch(() => {
      // The operation already finished; there is nothing left to cancel.
    });
  };

  const onConfirmRemove = () =>
    run("Could not remove model", "remove", async () => {
      if (!pendingRemove) return;
      const result = await aiRemoveModel(pendingRemove.checksum);
      setPendingRemove(null);
      if (preview?.sha256 === result.checksum) {
        clearPreview();
      }
      if (snapshot.selectedChecksum === result.checksum) {
        setReply("");
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
        <Badge variant={fakeBackend ? "warning" : loaded ? "success" : "neutral"}>
          {fakeBackend ? `Test backend · ${snapshot.backendStatus}` : snapshot.backendStatus}
        </Badge>
      </div>

      {fakeBackend && (
        <Alert variant="warning" title="Test backend only">
          This build does not include a real local-model runtime. Importing and loading exercise the
          lifecycle, but compatibility is reported as unsupported until a real runtime is
          installed.
        </Alert>
      )}

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
              {facts ? (
                <>
                  <Badge variant={facts.compatibilityTone}>{facts.compatibilityLabel}</Badge>
                  <div className="setting-row__desc">{facts.compatibility.reason}</div>
                </>
              ) : (
                "Choose a file to see its compatibility before importing."
              )}
            </dd>
          </div>
        </dl>
      )}

      {facts && (
        <Alert
          variant={facts.compatibilityTone}
          title={`Compatibility: ${facts.compatibilityLabel}`}
        >
          <p>{facts.compatibility.reason}</p>
          <p>{facts.compatibility.recommendation}</p>
          {facts.compatibility.requiredMemoryBytes === 0 && (
            <p>
              Imported files do not declare a memory requirement, so only the operating system,
              architecture, runtime, and free disk are checked.
            </p>
          )}
        </Alert>
      )}

      {snapshot.ready.length > 0 ? (
        <ul className="model-ready-list">
          {snapshot.ready.map((model) => {
            const isSelected = loaded && snapshot.selectedChecksum === model.checksum;
            return (
              <li key={model.checksum} className="model-ready">
                <div>
                  <div className="setting-row__label">
                    {model.checksum.slice(0, 8)}
                    <span className="model-ready__size"> {formatBytes(model.size)}</span>
                  </div>
                  <div className="setting-row__desc">
                    License: {licenseLabel(model.license)} · Runtime: {model.runtimeCompat}
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
      ) : (
        <div className="setting-row__desc">No model is installed. OffPDF still works normally.</div>
      )}

      {snapshot.storeRoot && (
        <div className="setting-row__desc mono">Store: {snapshot.storeRoot}</div>
      )}

      {loaded && (
        <div className="model-test">
          <Input
            label="Test the loaded model"
            value={prompt}
            onChange={(event) => setPrompt(event.target.value)}
            hint={
              fakeBackend
                ? "Runs entirely on this device. The test backend returns a deterministic reply."
                : "Runs entirely on this device. Nothing is sent over the network."
            }
            disabled={busy}
          />
          <div className="row gap-sm">
            <Button
              variant="secondary"
              onClick={onGenerate}
              loading={operation === "generate"}
              disabled={busy && operation !== "generate"}
            >
              Run test
            </Button>
            {operation === "generate" && (
              <Button variant="secondary" onClick={onCancel} leftIcon={<Icon name="stop" size={16} />}>
                Cancel
              </Button>
            )}
          </div>
          {reply && (
            <pre className="model-test__reply" aria-live="polite">
              {reply}
            </pre>
          )}
        </div>
      )}

      <div className="row gap-sm model-actions">
        <Button
          variant="primary"
          onClick={onImport}
          loading={operation === "import"}
          disabled={busy && operation !== "import"}
          leftIcon={<Icon name="upload" size={16} />}
        >
          {view.importCta}
        </Button>
        {preview && (
          <Button variant="secondary" onClick={onChooseDifferent} disabled={busy}>
            Choose a different file
          </Button>
        )}
        {preview && !busy && (
          <Button variant="ghost" onClick={clearPreview}>
            Clear preview
          </Button>
        )}
        {canCancel && (
          <Button variant="secondary" onClick={onCancel} leftIcon={<Icon name="stop" size={16} />}>
            Cancel
          </Button>
        )}
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
            <Button
              variant="danger"
              onClick={onConfirmRemove}
              loading={operation === "remove"}
              disabled={busy && operation !== "remove"}
            >
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
