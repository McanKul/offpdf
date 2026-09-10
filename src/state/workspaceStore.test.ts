import { beforeEach, describe, expect, it, vi } from "vitest";

const commands = vi.hoisted(() => ({
  getFileInfo: vi.fn(),
  imageToPdf: vi.fn(),
  officeToPdf: vi.fn(),
  copyFile: vi.fn(),
  mergePdfs: vi.fn(),
  compressPdf: vi.fn(),
  editPdf: vi.fn(),
  editPdfOverlays: vi.fn(),
}));

vi.mock("@/lib/tauriCommands", () => commands);

import { useWorkspace } from "./workspaceStore";

beforeEach(() => {
  vi.resetAllMocks();
  useWorkspace.setState({ files: [], activeIndex: 0, loading: false });
  commands.getFileInfo.mockImplementation(async (path: string) => ({
    path,
    name: path.split(/[\\/]/).pop() || path,
    sizeBytes: 100,
    pageCount: 1,
    isValidPdf: true,
  }));
  commands.officeToPdf.mockImplementation(async (path: string) => `${path}.pdf`);
});

describe("workspace image imports", () => {
  it("serializes conversions and reuses a conversion for duplicate image paths", async () => {
    let active = 0;
    let maxActive = 0;
    commands.imageToPdf.mockImplementation(async (path: string) => {
      active += 1;
      maxActive = Math.max(maxActive, active);
      await Promise.resolve();
      active -= 1;
      return `${path}.pdf`;
    });

    const result = await useWorkspace
      .getState()
      .addPaths(["/photos/a.heic", "/photos/b.heif", "/photos/a.heic"]);

    expect(maxActive).toBe(1);
    expect(commands.imageToPdf).toHaveBeenCalledTimes(2);
    expect(commands.imageToPdf).toHaveBeenNthCalledWith(1, "/photos/a.heic");
    expect(commands.imageToPdf).toHaveBeenNthCalledWith(2, "/photos/b.heif");
    expect(result.added).toBe(3);
    expect(result.errors).toEqual([]);
    expect(useWorkspace.getState().files).toHaveLength(3);
  });

  it("serializes image conversions across concurrent addPaths calls", async () => {
    let active = 0;
    let maxActive = 0;
    let resolveFirst!: () => void;
    let resolveSecond!: () => void;
    commands.imageToPdf
      .mockImplementationOnce(
        (path: string) =>
          new Promise<string>((resolve) => {
            active += 1;
            maxActive = Math.max(maxActive, active);
            resolveFirst = () => {
              active -= 1;
              resolve(`${path}.pdf`);
            };
          }),
      )
      .mockImplementationOnce(
        (path: string) =>
          new Promise<string>((resolve) => {
            active += 1;
            maxActive = Math.max(maxActive, active);
            resolveSecond = () => {
              active -= 1;
              resolve(`${path}.pdf`);
            };
          }),
      );

    const firstPromise = useWorkspace.getState().addPaths(["/photos/first.heic"]);
    const secondPromise = useWorkspace.getState().addPaths(["/photos/second.heif"]);
    await Promise.resolve();

    expect(useWorkspace.getState().loading).toBe(true);
    expect(commands.imageToPdf).toHaveBeenCalledTimes(1);
    resolveFirst();
    const first = await firstPromise;

    expect(useWorkspace.getState().loading).toBe(true);
    expect(commands.imageToPdf).toHaveBeenCalledTimes(2);
    resolveSecond();
    const second = await secondPromise;

    expect(maxActive).toBe(1);
    expect(commands.imageToPdf).toHaveBeenCalledTimes(2);
    expect(first.added).toBe(1);
    expect(second.added).toBe(1);
    expect(useWorkspace.getState().files).toHaveLength(2);
    expect(useWorkspace.getState().loading).toBe(false);
  });

  it("keeps importing after one image conversion returns an error", async () => {
    commands.imageToPdf
      .mockRejectedValueOnce(new Error("The image is incomplete."))
      .mockResolvedValueOnce("/tmp/good.pdf");

    const result = await useWorkspace
      .getState()
      .addPaths(["/photos/bad.heic", "/photos/good.heif"]);

    expect(result.added).toBe(1);
    expect(result.errors).toEqual(["bad.heic: The image is incomplete."]);
    expect(useWorkspace.getState().files).toHaveLength(1);
    expect(useWorkspace.getState().loading).toBe(false);
  });
});

describe("OS-open workspace intake", () => {
  it("rejects OS-open of report.pdf.exe and .zip without adding them", async () => {
    const result = await useWorkspace
      .getState()
      .addPaths(["/tmp/report.pdf.exe", "/tmp/archive.zip"]);

    expect(result.added).toBe(0);
    expect(result.notPdf).toBe(true);
    expect(result.errors).toEqual([]);
    expect(useWorkspace.getState().files).toHaveLength(0);
    expect(commands.getFileInfo).not.toHaveBeenCalled();
    expect(commands.copyFile).not.toHaveBeenCalled();
    expect(commands.mergePdfs).not.toHaveBeenCalled();
    expect(commands.compressPdf).not.toHaveBeenCalled();
    expect(commands.editPdf).not.toHaveBeenCalled();
    expect(commands.editPdfOverlays).not.toHaveBeenCalled();
  });

  it("keeps a supported path when mixed with unsupported OS-open files", async () => {
    const result = await useWorkspace
      .getState()
      .addPaths(["/tmp/report.pdf.exe", "/tmp/opened.pdf", "/tmp/archive.zip"]);

    expect(result.added).toBe(1);
    expect(result.notPdf).toBe(true);
    expect(result.errors).toEqual([]);
    expect(useWorkspace.getState().files).toHaveLength(1);
    expect(useWorkspace.getState().files[0]?.path).toBe("/tmp/opened.pdf");
    expect(commands.getFileInfo).toHaveBeenCalledTimes(1);
    expect(commands.getFileInfo).toHaveBeenCalledWith("/tmp/opened.pdf");
  });

  it("collects a missing OS-open path as an error and does not write a dest", async () => {
    commands.getFileInfo.mockRejectedValueOnce({
      code: "IO",
      title: "Could not read file",
      message: "Could not read file information.",
    });

    const missing = "/tmp/does-not-exist-os-open.pdf";
    const result = await useWorkspace.getState().addPaths([missing]);

    expect(result.added).toBe(0);
    expect(result.notPdf).toBe(false);
    expect(result.errors).toEqual([
      "does-not-exist-os-open.pdf: Could not read file information.",
    ]);
    expect(useWorkspace.getState().files).toHaveLength(0);
    expect(commands.copyFile).not.toHaveBeenCalled();
    expect(commands.mergePdfs).not.toHaveBeenCalled();
    expect(commands.compressPdf).not.toHaveBeenCalled();
    expect(commands.editPdf).not.toHaveBeenCalled();
    expect(commands.editPdfOverlays).not.toHaveBeenCalled();
    expect(commands.imageToPdf).not.toHaveBeenCalled();
    expect(commands.officeToPdf).not.toHaveBeenCalled();
  });

  it("OS-open intake addPaths does not start merge, compress, or redact", async () => {
    const result = await useWorkspace.getState().addPaths(["/tmp/opened.pdf"]);

    expect(result.added).toBe(1);
    expect(result.notPdf).toBe(false);
    expect(result.errors).toEqual([]);
    expect(commands.getFileInfo).toHaveBeenCalledWith("/tmp/opened.pdf");
    expect(commands.mergePdfs).not.toHaveBeenCalled();
    expect(commands.compressPdf).not.toHaveBeenCalled();
    expect(commands.editPdf).not.toHaveBeenCalled();
    expect(commands.editPdfOverlays).not.toHaveBeenCalled();
    expect(commands.copyFile).not.toHaveBeenCalled();
  });
});
