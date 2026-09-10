import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { TOOLS } from "@/lib/tools";
import { HomeToolsSection } from "./HomeToolsSection";

const FEATURED_IDS = new Set(["merge", "compress", "reorder"]);
const noop = () => {};

function renderSection(query: string, category: "All" | "Convert" | "Organize" = "All") {
  return renderToStaticMarkup(
    <HomeToolsSection
      query={query}
      onQueryChange={noop}
      category={category}
      onCategory={noop}
      onOpen={noop}
    />,
  );
}

function getAttribute(tag: string, name: string) {
  return tag.match(new RegExp(`\\s${name}="([^"]+)"`))?.[1];
}

function classIndex(markup: string, className: string) {
  const match = markup.match(new RegExp(`class="[^"]*\\b${className}\\b[^"]*"`));
  return match ? markup.indexOf(match[0]) : -1;
}

function toolSearchInput(markup: string) {
  const searchIndex = classIndex(markup, "tool-search");
  const pillsIndex = classIndex(markup, "category-pills");
  if (searchIndex < 0 || pillsIndex < 0 || pillsIndex <= searchIndex) return "";
  return markup.slice(searchIndex, pillsIndex).match(/<input\b[^>]*>/)?.[0] ?? "";
}

function labelledText(markup: string, id: string) {
  const labelled = markup.match(new RegExp(`\\sid="${id}"[^>]*>([^<]*)`));
  if (labelled?.[1]?.trim()) return labelled[1].trim();
  const forLabel = markup.match(new RegExp(`<label[^>]*\\sfor="${id}"[^>]*>([\\s\\S]*?)</label>`));
  return forLabel ? forLabel[1].replace(/<[^>]+>/g, "").trim() : "";
}

function inputAccessibleName(markup: string, inputTag: string) {
  const ariaLabel = getAttribute(inputTag, "aria-label")?.trim();
  if (ariaLabel) return ariaLabel;

  const labelledBy = getAttribute(inputTag, "aria-labelledby");
  if (labelledBy) {
    const fromId = labelledText(markup, labelledBy);
    if (fromId) return fromId;
  }

  const id = getAttribute(inputTag, "id");
  if (id) {
    const fromFor = labelledText(markup, id);
    if (fromFor) return fromFor;
  }

  const wrap = markup.match(new RegExp(`<label\\b[^>]*>([\\s\\S]*?)${inputTag.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}`));
  return wrap ? wrap[1].replace(/<[^>]+>/g, "").trim() : "";
}

function controlAccessibleName(tag: string, inner: string, markup: string) {
  const ariaLabel = getAttribute(tag, "aria-label")?.trim();
  if (ariaLabel) return ariaLabel;
  const labelledBy = getAttribute(tag, "aria-labelledby");
  if (labelledBy) {
    const fromId = labelledText(markup, labelledBy);
    if (fromId) return fromId;
  }
  return inner.replace(/<[^>]+>/g, "").trim();
}

function clearControls(markup: string) {
  const buttons = [...markup.matchAll(/<button\b([^>]*)>([\s\S]*?)<\/button>/g)].map(([, attrs, inner]) =>
    controlAccessibleName(`<button${attrs}>`, inner, markup),
  );
  const roleButtons = [...markup.matchAll(/<([a-z]+)([^>]*\srole="button"[^>]*)>([\s\S]*?)<\/\1>/gi)].map(
    ([, , attrs, inner]) => controlAccessibleName(`<x${attrs}>`, inner, markup),
  );
  const inputs = [...markup.matchAll(/<input\b[^>]*>/g)]
    .map(([tag]) => tag)
    .filter((tag) => /^(button|reset)$/i.test(getAttribute(tag, "type") ?? ""))
    .map((tag) => controlAccessibleName(tag, "", markup));
  return [...buttons, ...roleButtons, ...inputs].filter((name) => /clear/i.test(name));
}

describe("HomeToolsSection", () => {
  it("HS-FIELD: search input inside .tool-search appears above .category-pills", () => {
    const markup = renderSection("");
    const searchIndex = classIndex(markup, "tool-search");
    const pillsIndex = classIndex(markup, "category-pills");

    expect(searchIndex).toBeGreaterThan(-1);
    expect(pillsIndex).toBeGreaterThan(searchIndex);
    expect(toolSearchInput(markup)).toMatch(/<input\b/);
  });

  it("HS-LABEL: the search input has a non-empty accessible name", () => {
    const markup = renderSection("");
    const input = toolSearchInput(markup);
    expect(input).toMatch(/<input\b/);
    expect(inputAccessibleName(markup, input).length).toBeGreaterThan(0);
  });

  it("HS-SSR-EMPTY: empty query + All still has featured-strip and non-featured tool names", () => {
    const markup = renderSection("");
    const nonFeatured = TOOLS.filter((tool) => !FEATURED_IDS.has(tool.id));

    expect(classIndex(markup, "featured-strip")).toBeGreaterThan(-1);
    expect(nonFeatured.length).toBeGreaterThan(0);
    for (const tool of nonFeatured) {
      expect(markup).toContain(tool.name);
    }
  });

  it('HS-SSR-FILTER: query "docx" shows Office / HTML to PDF and hides Unlock PDF', () => {
    const markup = renderSection("docx");
    expect(markup).toContain("Office / HTML to PDF");
    expect(markup).not.toContain("Unlock PDF");
  });

  it('HS-SSR-EMPTY-STATE: nonsense query has a no-results node and no .tool-card', () => {
    const markup = renderSection("zzzxxyyzz");
    expect(markup).toMatch(/class="[^"]*no-results[^"]*"/);
    expect(classIndex(markup, "tool-card")).toBe(-1);
  });

  it("HS-CLEAR-CONTROL: non-empty query exposes a /clear/i control; empty query does not", () => {
    expect(clearControls(renderSection("docx")).length).toBeGreaterThan(0);
    expect(clearControls(renderSection(""))).toEqual([]);
  });
});
