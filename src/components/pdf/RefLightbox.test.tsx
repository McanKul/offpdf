import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { PageRef } from "@/lib/types";
import { RefLightbox } from "./RefLightbox";

const noop = () => {};

describe("RefLightbox accessibility", () => {
  it("keeps zoom controls out of the dialog's accessible title", () => {
    const ref: PageRef = {
      key: "sample.pdf#3",
      path: "/tmp/sample.pdf",
      fileName: "sample.pdf",
      page: 3,
    };
    const markup = renderToStaticMarkup(
      <RefLightbox list={[ref]} current={ref} onClose={noop} />,
    );
    const labelledBy = markup.match(/role="dialog"[^>]*aria-labelledby="([^"]+)"/)?.[1];
    const titles = [...markup.matchAll(/<div class="modal__title" id="([^"]+)">(.*?)<\/div>/g)];
    const labelledTitle = titles.find(([, id]) => id === labelledBy)?.[2];

    expect(labelledBy).toBeTruthy();
    expect(labelledTitle).toContain("sample.pdf · page 1 of 1");
    expect(labelledTitle).not.toContain("100%");
    expect(labelledTitle).not.toContain("Zoom out");
    expect(labelledTitle).not.toContain("Reset zoom");
    expect(labelledTitle).not.toContain("Zoom in");
    expect(markup).toContain('aria-label="Zoom out"');
    expect(markup).toContain('aria-label="Reset zoom, current zoom 100%"');
    expect(markup).toContain('aria-label="Zoom in"');
  });
});
