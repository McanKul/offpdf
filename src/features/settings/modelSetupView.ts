import { formatBytes } from "@/lib/formatBytes";

const HARDWARE_STUB = "Hardware check is not available in this version.";

export interface ModelSetupReady {
  checksum: string;
  size: number;
  license?: string;
  runtimeCompat?: string;
}

export interface ModelSetupSnapshotInput {
  ready: ModelSetupReady[];
  backendStatus?: string;
  storeRoot?: string;
  selectedChecksum?: string | null;
}

export interface ModelSetupPreviewInput {
  size: number;
  sha256?: string;
  license: string;
  runtimeCompat: string;
  location: string;
  compatibility?: string;
}

export interface ModelSetupPendingRemove {
  checksum: string;
  size: number;
}

export interface ModelSetupFacts {
  size: number;
  license: string;
  location: string;
  compatibility: string;
  imported: boolean;
}

export interface ModelSetupView {
  showFacts: boolean;
  importCta: string;
  downloadUrl: string | null;
  autoImport: boolean;
  facts?: ModelSetupFacts;
  confirmRemove?: string;
}

export function modelSetupView(input: {
  snapshot: ModelSetupSnapshotInput;
  preview?: ModelSetupPreviewInput;
  pendingRemove?: ModelSetupPendingRemove;
}): ModelSetupView {
  const preview = input.preview;
  const facts = preview
    ? {
        size: preview.size,
        license: preview.license,
        location: preview.location,
        compatibility: composeCompatibility(preview.runtimeCompat, preview.compatibility),
        imported: false,
      }
    : undefined;

  const pending = input.pendingRemove;
  const confirmRemove = pending
    ? `Remove model ${pending.checksum.slice(0, 8)} (${formatBytes(pending.size)})? This deletes the imported copy from OffPDF app data.`
    : undefined;

  return {
    showFacts: true,
    importCta: preview ? "Import this file" : "Import",
    downloadUrl: null,
    autoImport: false,
    facts,
    confirmRemove,
  };
}

function composeCompatibility(runtimeCompat: string, compatibility?: string): string {
  const composed = compatibility?.trim();
  if (composed) return composed;
  const runtime = runtimeCompat.trim();
  return runtime ? `${runtime} — ${HARDWARE_STUB}` : HARDWARE_STUB;
}
