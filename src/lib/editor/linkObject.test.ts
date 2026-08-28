import { describe, it, expect } from "vitest";
import { makeLinkObject } from "./editReducer";

describe("makeLinkObject", () => {
  it("stores unrotated rect and uri / goto action", () => {
    const uri = makeLinkObject("l1", 0, { x: 100, y: 200, w: 80, h: 40 }, {
      type: "uri",
      uri: "https://example.com",
    });
    expect(uri.kind).toBe("link");
    expect(uri.pageIndex).toBe(0);
    expect(uri.rect).toEqual({ x: 100, y: 200, w: 80, h: 40 });
    expect(uri.action).toEqual({ type: "uri", uri: "https://example.com" });

    const goto = makeLinkObject("l2", 1, { x: 10, y: 20, w: 30, h: 40 }, {
      type: "goto",
      destPageIndex: 2,
    });
    expect(goto.kind).toBe("link");
    expect(goto.action).toEqual({ type: "goto", destPageIndex: 2 });
    expect(JSON.parse(JSON.stringify(goto)).kind).toBe("link");
  });
});
