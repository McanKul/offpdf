import { beforeEach, describe, expect, it, vi } from "vitest";

const commands = vi.hoisted(() => ({
  getFileInfo: vi.fn(),
  imageToPdf: vi.fn(),
  officeToPdf: vi.fn(),
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
