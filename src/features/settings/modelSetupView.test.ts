import { describe, expect, it } from "vitest";
import { formatBytes } from "@/lib/formatBytes";
import { TOOLS } from "@/lib/tools";

const FIXTURE_SHA256 =
  "9a32fd0b4e68056601b986dc617f8d3be37fe0af3903d98cdda02fff552c884e";
const FIXTURE_SIZE = 23;
const HARDWARE_STUB = "Hardware check is not available in this version.";
const STORE_LOCATION = `/tmp/offpdf-test-store/blobs/${FIXTURE_SHA256}`;

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

  it("preview facts include size, license, location, and the hardware stub before imported is true", async () => {
    const { modelSetupView } = await loadView();
    const view = modelSetupView({
      snapshot: { ready: [], backendStatus: "Unloaded" },
      preview: {
        size: FIXTURE_SIZE,
        sha256: FIXTURE_SHA256,
        license: "imported",
        runtimeCompat: "any",
        location: STORE_LOCATION,
      },
    });

    expect(view.facts).toBeTruthy();
    expect(view.facts?.size).toBe(FIXTURE_SIZE);
    expect(view.facts?.license).toBe("imported");
    expect(view.facts?.location).toBe(STORE_LOCATION);
    expect(view.facts?.compatibility).toContain("any");
    expect(view.facts?.compatibility).toContain(HARDWARE_STUB);
    expect(view.facts?.imported).toBe(false);
    expect(view.downloadUrl ?? null).toBeNull();
    expect(view.autoImport).toBe(false);
  });
});

describe("setup-facts-wire-compat", () => {
  it("live DTO compatibility contains the hardware stub once and does not start with any — any", async () => {
    const { modelSetupView } = await loadView();
    const view = modelSetupView({
      snapshot: { ready: [], backendStatus: "Unloaded" },
      preview: {
        size: FIXTURE_SIZE,
        license: "imported",
        location: STORE_LOCATION,
        runtimeCompat: "any",
        compatibility: `any. ${HARDWARE_STUB}`,
      },
    });

    const compatibility = view.facts?.compatibility ?? "";
    expect(compatibility.split(HARDWARE_STUB).length - 1).toBe(1);
    expect(compatibility.startsWith("any — any")).toBe(false);
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
        license: "imported",
        location: STORE_LOCATION,
        runtimeCompat: "any",
        compatibility: `any. ${HARDWARE_STUB}`,
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
  it("remove-confirm copy names the checksum prefix and formatBytes(size)", async () => {
    const { modelSetupView } = await loadView();
    const view = modelSetupView({
      snapshot: {
        ready: [{ checksum: FIXTURE_SHA256, size: FIXTURE_SIZE }],
        backendStatus: "Unloaded",
      },
      pendingRemove: { checksum: FIXTURE_SHA256, size: FIXTURE_SIZE },
    });

    expect(view.confirmRemove).toBeTruthy();
    expect(view.confirmRemove).toContain(FIXTURE_SHA256.slice(0, 8));
    expect(view.confirmRemove).toContain(formatBytes(FIXTURE_SIZE));
  });
});
