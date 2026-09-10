import { describe, expect, it } from "vitest";
import { homeSearchEscapeAction } from "./homeSearchEscape";

describe("homeSearchEscapeAction", () => {
  it("HS-ESCAPE: Escape clears then blurs; other keys do not", () => {
    expect(homeSearchEscapeAction({ key: "Escape" })).toEqual({ clear: true, blur: true });
    expect(homeSearchEscapeAction({ key: "Enter" })).toEqual({ clear: false, blur: false });
    expect(homeSearchEscapeAction({ key: "k" })).toEqual({ clear: false, blur: false });
    expect(homeSearchEscapeAction({ key: "Backspace" })).toEqual({ clear: false, blur: false });
  });
});
