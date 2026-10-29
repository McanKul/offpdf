import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { ToastProvider } from "@/components/ui/Toast";
import { SettingsPage } from "./SettingsPage";

vi.mock("@tauri-apps/api/app", () => ({
  getVersion: async () => "0.5.0",
}));

vi.mock("@/lib/tauriCommands", () => ({
  clearTempFiles: async () => 0,
  getTempDir: async () => "",
  openPath: async () => undefined,
  aiCancel: async () => undefined,
  aiGenerate: async () => "",
  aiImportModel: async () => ({}),
  aiListModels: async () => ({
    ready: [],
    backendStatus: "Unloaded",
    health: { ok: false, backendId: "fake" },
    selectedChecksum: null,
    storeRoot: "",
  }),
  aiLoadModel: async () => undefined,
  aiPickModelFile: async () => null,
  aiPreviewModel: async () => ({}),
  aiRemoveModel: async () => ({ checksum: "", recoveredBytes: 0 }),
  aiUnloadModel: async () => undefined,
}));

describe("SettingsPage local model section", () => {
  it("mounts the opt-in local model controls without a model installed", () => {
    const html = renderToStaticMarkup(
      <ToastProvider>
        <SettingsPage />
      </ToastProvider>,
    );

    expect(html).toContain("Local model");
    expect(html).toContain("Test backend");
    expect(html).toContain("No model is installed");
    expect(html).toContain("Choose a file to see its compatibility");
    expect(html).not.toContain("autoImport");
  });
});
