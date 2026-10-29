import { formatBytes } from "@/lib/formatBytes";
import type { CompatibilityReport, CompatibilityStatus } from "@/lib/types";

export type ModelSetupTone = "success" | "warning" | "danger";

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
  compatibility: CompatibilityReport;
}

export interface ModelSetupPendingRemove {
  checksum: string;
  size: number;
}

export interface ModelSetupFacts {
  size: number;
  license: string;
  runtimeCompat: string;
  location: string;
  compatibility: CompatibilityReport;
  compatibilityLabel: string;
  compatibilityTone: ModelSetupTone;
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

const UNKNOWN_LICENSE = "Unknown — check the model's license before using it.";

export function modelSetupView(input: {
  snapshot: ModelSetupSnapshotInput;
  preview?: ModelSetupPreviewInput;
  pendingRemove?: ModelSetupPendingRemove;
}): ModelSetupView {
  const preview = input.preview;
  const facts = preview
    ? {
        size: preview.size,
        license: licenseLabel(preview.license),
        runtimeCompat: preview.runtimeCompat,
        location: preview.location,
        compatibility: preview.compatibility,
        compatibilityLabel: compatibilityLabel(preview.compatibility.status),
        compatibilityTone: compatibilityTone(preview.compatibility.status),
        imported: false,
      }
    : undefined;

  const pending = input.pendingRemove;
  const confirmRemove = pending
    ? `Remove model ${pending.checksum} (${formatBytes(pending.size)})? This deletes the imported copy from OffPDF app data.`
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

/** Imported files often have no declared license; never present a placeholder as one. */
export function licenseLabel(license: string | undefined): string {
  const value = license?.trim();
  if (!value || value === "imported" || value.toLowerCase() === "unknown") {
    return UNKNOWN_LICENSE;
  }
  return value;
}

export function compatibilityLabel(status: CompatibilityStatus): string {
  switch (status) {
    case "supported":
      return "Supported";
    case "supportedWithLimits":
      return "Supported with limits";
    case "unsupported":
      return "Unsupported";
  }
}

export function compatibilityTone(status: CompatibilityStatus): ModelSetupTone {
  switch (status) {
    case "supported":
      return "success";
    case "supportedWithLimits":
      return "warning";
    case "unsupported":
      return "danger";
  }
}
