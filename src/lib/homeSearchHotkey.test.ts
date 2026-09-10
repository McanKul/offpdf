import { describe, expect, it } from "vitest";
import { isHomeSearchFocusHotkey } from "./homeSearchHotkey";

describe("isHomeSearchFocusHotkey", () => {
  it("HS-HOTKEY: Ctrl+K and Cmd+K are true; plain k and Ctrl+Alt+K are false", () => {
    expect(isHomeSearchFocusHotkey({ key: "k", ctrlKey: true, metaKey: false, altKey: false })).toBe(true);
    expect(isHomeSearchFocusHotkey({ key: "K", ctrlKey: true, metaKey: false, altKey: false })).toBe(true);
    expect(isHomeSearchFocusHotkey({ key: "k", ctrlKey: false, metaKey: true, altKey: false })).toBe(true);
    expect(isHomeSearchFocusHotkey({ key: "K", ctrlKey: false, metaKey: true, altKey: false })).toBe(true);

    expect(isHomeSearchFocusHotkey({ key: "k", ctrlKey: false, metaKey: false, altKey: false })).toBe(false);
    expect(isHomeSearchFocusHotkey({ key: "k", ctrlKey: true, metaKey: false, altKey: true })).toBe(false);
  });
});
