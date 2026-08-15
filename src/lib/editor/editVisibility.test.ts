import { describe, it, expect } from "vitest";
import { shouldShowEditCanvas } from "./editVisibility";

describe("shouldShowEditCanvas", () => {
  it("P2: inTauri=false is hidden (browser warning is separate)", () => {
    expect(shouldShowEditCanvas(1, 1, false)).toBe("hidden");
    expect(shouldShowEditCanvas(1, 0, false)).toBe("hidden");
    expect(shouldShowEditCanvas(0, 0, false)).toBe("hidden");
  });

  it("P2: Tauri + files + zero refs is no-pages, not a silent gap", () => {
    expect(shouldShowEditCanvas(1, 0, true)).toBe("no-pages");
  });

  it("P1: Tauri + refs is edit", () => {
    expect(shouldShowEditCanvas(1, 1, true)).toBe("edit");
    expect(shouldShowEditCanvas(2, 3, true)).toBe("edit");
  });

  it("P2: Tauri + no files is hidden", () => {
    expect(shouldShowEditCanvas(0, 0, true)).toBe("hidden");
  });
});
