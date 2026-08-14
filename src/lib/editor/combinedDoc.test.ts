import { describe, it, expect } from "vitest";
import { pageKeysForFiles } from "@/components/pdf/useCombinedDoc";
import type { WorkspaceFile } from "@/lib/types";
import { shouldShowEditCanvas } from "./editVisibility";

function wsFile(uid: string, pageCount: number | null | undefined): WorkspaceFile {
  const file: WorkspaceFile = {
    uid,
    path: `/tmp/${uid}.pdf`,
    name: `${uid}.pdf`,
    sizeBytes: 885,
    isValidPdf: true,
  };
  if (pageCount !== undefined) file.pageCount = pageCount;
  return file;
}

describe("pageKeysForFiles", () => {
  it("P1: pageCount >= 1 yields refs so the Edit card can mount", () => {
    const keys = pageKeysForFiles([wsFile("qa", 1)]);
    expect(keys).toEqual(["qa#1"]);
    expect(shouldShowEditCanvas(1, keys.length, true)).toBe("edit");
  });

  it("P4: a one-page file yields exactly one key, not an empty list", () => {
    expect(pageKeysForFiles([wsFile("one", 1)])).toEqual(["one#1"]);
  });

  it("P4: a two-page file yields two keys", () => {
    expect(pageKeysForFiles([wsFile("two", 2)])).toEqual(["two#1", "two#2"]);
  });

  it("P2: pageCount 0 produces no keys (current useCombinedDoc)", () => {
    expect(pageKeysForFiles([wsFile("empty", 0)])).toEqual([]);
  });

  it("P2: pageCount null produces no keys (current useCombinedDoc)", () => {
    expect(pageKeysForFiles([wsFile("nullish", null)])).toEqual([]);
  });

  it("P2: omitted pageCount produces no keys (current useCombinedDoc)", () => {
    expect(pageKeysForFiles([wsFile("missing", undefined)])).toEqual([]);
  });
});
