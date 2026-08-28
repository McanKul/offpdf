import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { Input } from "./Input";
import { Select } from "./Select";

function getAttribute(tag: string, name: string) {
  return tag.match(new RegExp(`\\s${name}="([^"]+)"`))?.[1];
}

describe("form control accessibility", () => {
  it("associates an Input label with an explicit id", () => {
    const markup = renderToStaticMarkup(<Input label="Email" id="email" />);
    const label = markup.match(/<label[^>]*>/)?.[0];
    const input = markup.match(/<input[^>]*>/)?.[0];

    expect(getAttribute(label ?? "", "for")).toBe("email");
    expect(getAttribute(input ?? "", "id")).toBe("email");
  });

  it("uses the Input name as its control id when no explicit id is provided", () => {
    const markup = renderToStaticMarkup(<Input label="Email" name="email" />);
    const label = markup.match(/<label[^>]*>/)?.[0];
    const input = markup.match(/<input[^>]*>/)?.[0];

    expect(getAttribute(label ?? "", "for")).toBe("email");
    expect(getAttribute(input ?? "", "id")).toBe("email");
  });

  it("generates unique ids for adjacent unlabeled controls", () => {
    const markup = renderToStaticMarkup(
      <>
        <Input label="First" />
        <Input label="Second" />
        <Select label="Third" options={[{ value: "one", label: "One" }]} value="one" onChange={() => {}} />
      </>,
    );
    const labels = [...markup.matchAll(/<label[^>]*>/g)].map(([tag]) => getAttribute(tag, "for"));
    const controls = [...markup.matchAll(/<(?:input|select)[^>]*>/g)].map(([tag]) => getAttribute(tag, "id"));

    expect(labels.every(Boolean)).toBe(true);
    expect(controls).toEqual(labels);
    expect(new Set(controls).size).toBe(3);
  });

  it("associates a Select label and preserves its name", () => {
    const markup = renderToStaticMarkup(
      <Select label="Format" name="format" options={[{ value: "pdf", label: "PDF" }]} value="pdf" onChange={() => {}} />,
    );
    const label = markup.match(/<label[^>]*>/)?.[0];
    const select = markup.match(/<select[^>]*>/)?.[0];

    expect(getAttribute(label ?? "", "for")).toBe("format");
    expect(getAttribute(select ?? "", "id")).toBe("format");
    expect(getAttribute(select ?? "", "name")).toBe("format");
  });
});
