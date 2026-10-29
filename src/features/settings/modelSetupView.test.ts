import { describe, expect, it } from "vitest";
import { formatBytes } from "@/lib/formatBytes";
import { TOOLS } from "@/lib/tools";
import type { CompatibilityReport } from "@/lib/types";

const FIXTURE_SHA256 =
  "9a32fd0b4e68056601b986dc617f8d3be37fe0af3903d98cdda02fff552c884e";
const FIXTURE_SIZE = 23;
const STORE_LOCATION = `/tmp/offpdf-test-store/blobs/${FIXTURE_SHA256}`;

const UNSUPPORTED: CompatibilityReport = {
  status: "unsupported",
  reason: "This build only includes the test inference backend.",
  recommendation: "Install a real local runtime before loading a model.",
  operatingSystem: "windows",
  architecture: "x86_64",
  runtimeId: "fake",
  runtimeAvailable: false,
  requiredMemoryBytes: 0,
  totalMemoryBytes: 16_000_000_000,
  availableMemoryBytes: 8_000_000_000,
  requiredDiskBytes: FIXTURE_SIZE,
  availableDiskBytes: 1_000_000_000,
};

const SUPPORTED: CompatibilityReport = {
  ...UNSUPPORTED,
  status: "supported",
  reason: "The model meets the local hardware requirements.",
  recommendation: "You can load this model.",
  runtimeId: "test-runtime",
  runtimeAvailable: true,
};

async function loadView() {
  return import("./modelSetupView");
}

describe("setup-zero-models-tools-ok", () => {
  it("keeps all 23 PDF tools registered with zero models", () => {
    expect(TOOLS).toHaveLength(23);
    expect(TOOLS.some((tool) => tool.id === "merge")).toBe(true);
    expect(TOOLS.some((tool) => tool.id === "ocr")).toBe(true);
    expect(TOOLS.every((tool) => tool.path.startsWith("/tools/"))).toBe(true);
  });
});

describe("setup-facts-before-import", () => {
  it("empty snapshot still shows a facts block and an Import CTA, with no download URL or auto-import", async () => {
    const { modelSetupView } = await loadView();
    const view = modelSetupView({
      snapshot: { ready: [], backendStatus: "Unloaded" },
    });

    expect(view.showFacts).toBe(true);
    expect(view.importCta).toMatch(/import/i);
    expect(view.downloadUrl ?? null).toBeNull();
    expect(view.autoImport).toBe(false);
  });

  it("preview facts include size, honest license, location, and structured compatibility before import", async () => {
    const { modelSetupView } = await loadView();
    const view = modelSetupView({
      snapshot: { ready: [], backendStatus: "Unloaded" },
      preview: {
        size: FIXTURE_SIZE,
        sha256: FIXTURE_SHA256,
        license: "Unknown",
        runtimeCompat: "any",
        location: STORE_LOCATION,
        compatibility: UNSUPPORTED,
      },
    });

    expect(view.facts).toBeTruthy();
    expect(view.facts?.size).toBe(FIXTURE_SIZE);
    expect(view.facts?.license).toContain("Unknown");
    expect(view.facts?.license).not.toBe("imported");
    expect(view.facts?.location).toBe(STORE_LOCATION);
    expect(view.facts?.compatibility.status).toBe("unsupported");
    expect(view.facts?.compatibility.reason).toContain("test inference backend");
    expect(view.facts?.compatibilityLabel).toBe("Unsupported");
    expect(view.facts?.compatibilityTone).toBe("danger");
    expect(view.facts?.imported).toBe(false);
    expect(view.importCta).toBe("Import this file");
    expect(view.downloadUrl ?? null).toBeNull();
    expect(view.autoImport).toBe(false);
  });

  it("legacy imported manifests are not presented as a license", async () => {
    const { licenseLabel } = await loadView();

    expect(licenseLabel("imported")).toContain("Unknown");
    expect(licenseLabel("")).toContain("Unknown");
    expect(licenseLabel("CC0-1.0")).toBe("CC0-1.0");
  });
});

describe("setup-facts-wire-compat", () => {
  it("uses the live compatibility report status and recommendation", async () => {
    const { modelSetupView } = await loadView();
    const view = modelSetupView({
      snapshot: { ready: [], backendStatus: "Unloaded" },
      preview: {
        size: FIXTURE_SIZE,
        license: "Unknown",
        location: STORE_LOCATION,
        runtimeCompat: "any",
        compatibility: SUPPORTED,
      },
    });

    expect(view.facts?.compatibility.status).toBe("supported");
    expect(view.facts?.compatibility.reason).toBe(SUPPORTED.reason);
    expect(view.facts?.compatibilityLabel).toBe("Supported");
    expect(view.facts?.compatibilityTone).toBe("success");
    expect(view.showFacts).toBe(true);
    expect(view.importCta).toMatch(/import/i);
  });
});

describe("setup-preview-can-rechoose", () => {
  it("preview keeps the facts panel and an Import CTA so another file can be chosen", async () => {
    const { modelSetupView } = await loadView();
    const view = modelSetupView({
      snapshot: { ready: [], backendStatus: "Unloaded" },
      preview: {
        size: FIXTURE_SIZE,
        license: "Unknown",
        location: STORE_LOCATION,
        runtimeCompat: "any",
        compatibility: UNSUPPORTED,
      },
    });

    expect(view.showFacts).toBe(true);
    expect(view.importCta).toMatch(/import/i);
    expect(view.downloadUrl ?? null).toBeNull();
    expect(view.autoImport).toBe(false);
  });
});

describe("setup-import-explicit-only", () => {
  it("empty-store view has no download URL and does not auto-import", async () => {
    const { modelSetupView } = await loadView();
    const view = modelSetupView({
      snapshot: { ready: [] },
    });

    expect(view.downloadUrl ?? null).toBeNull();
    expect(view.autoImport).toBe(false);
    expect(view.importCta).toMatch(/import/i);
  });
});

describe("setup-remove-reports-bytes", () => {
  it("remove-confirm copy names the full target checksum and formatBytes(size)", async () => {
    const { modelSetupView } = await loadView();
    const view = modelSetupView({
      snapshot: {
        ready: [{ checksum: FIXTURE_SHA256, size: FIXTURE_SIZE }],
        backendStatus: "Unloaded",
      },
      pendingRemove: { checksum: FIXTURE_SHA256, size: FIXTURE_SIZE },
    });

    expect(view.confirmRemove).toBeTruthy();
    expect(view.confirmRemove).toContain(FIXTURE_SHA256);
    expect(view.confirmRemove).toContain(formatBytes(FIXTURE_SIZE));
  });
});
