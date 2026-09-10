// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { FormField } from "@/lib/editor";
import type { PageLayout } from "./PageSurface";
import { FormFieldsOverlay } from "./FormFieldsOverlay";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT =
  true;

const noop = () => {};

/** Letter page, same geometry as `coords.test.ts` (box 612×792, rotate 0). */
const letterLayout: PageLayout = {
  cssWidth: 612,
  cssHeight: 792,
  geometry: {
    box: { x: 0, y: 0, w: 612, h: 792 },
    rotate: 0,
    pageIndex: 0,
  },
};

const LETTER_RECT = { x: 72, y: 680, w: 200, h: 24 };

function field(overrides: Partial<FormField> & Pick<FormField, "name" | "kind">): FormField {
  return {
    pageIndex: 0,
    rect: LETTER_RECT,
    value: null,
    exportValues: [],
    choices: [],
    readOnly: false,
    hidden: false,
    maxLen: null,
    multiline: false,
    comboEdit: false,
    ...overrides,
  };
}

function renderOverlay(
  fields: FormField[],
  values: Record<string, string> = {},
  sourcePage = 1,
): string {
  return renderToStaticMarkup(
    <FormFieldsOverlay
      layout={letterLayout}
      fields={fields}
      values={values}
      sourcePage={sourcePage}
      onChange={noop}
    />,
  );
}

function getAttribute(tag: string, name: string): string | undefined {
  return tag.match(new RegExp(`\\s${name}="([^"]*)"`))?.[1];
}

function hasBooleanAttr(tag: string, name: string): boolean {
  return new RegExp(`\\s${name}(?:="[^"]*")?(?=[\\s/>])`).test(tag);
}

function controlTags(markup: string): string[] {
  return [...markup.matchAll(/<(input|textarea|select)\b[^>]*>/g)].map((m) => m[0]);
}

function controlByAriaLabel(markup: string, name: string): string | undefined {
  return controlTags(markup).find((tag) => getAttribute(tag, "aria-label") === name);
}

function selectBlock(markup: string, name: string): string | undefined {
  for (const match of markup.matchAll(/<select\b([^>]*)>([\s\S]*?)<\/select>/g)) {
    if (getAttribute(`<select${match[1]}>`, "aria-label") === name) return match[0];
  }
  return undefined;
}

function optionValues(html: string): string[] {
  return [...html.matchAll(/<option\b([^>]*)>([^<]*)<\/option>/g)].map((match) => {
    return getAttribute(`<option${match[1]}>`, "value") ?? match[2];
  });
}

function datalistBlock(markup: string): string | undefined {
  return markup.match(/<datalist\b[^>]*>[\s\S]*?<\/datalist>/)?.[0];
}

function textareaBody(markup: string, name: string): string | undefined {
  for (const match of markup.matchAll(/<textarea\b([^>]*)>([\s\S]*?)<\/textarea>/g)) {
    if (getAttribute(`<textarea${match[1]}>`, "aria-label") === name) return match[2];
  }
  return undefined;
}

type ChangeSpy = ReturnType<typeof vi.fn<(name: string, value: string) => void>>;

let live: { root: Root; container: HTMLDivElement } | null = null;

afterEach(() => {
  if (!live) return;
  const { root, container } = live;
  live = null;
  act(() => {
    root.unmount();
  });
  container.remove();
});

function mountOverlay(
  fields: FormField[],
  values: Record<string, string> = {},
  sourcePage = 1,
): { container: HTMLDivElement; onChange: ChangeSpy } {
  const onChange = vi.fn<(name: string, value: string) => void>();
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  live = { root, container };
  act(() => {
    root.render(
      <FormFieldsOverlay
        layout={letterLayout}
        fields={fields}
        values={values}
        sourcePage={sourcePage}
        onChange={onChange}
      />,
    );
  });
  return { container, onChange };
}

function controlByAriaName<T extends Element>(container: HTMLElement, name: string): T {
  const el = container.querySelector(`[aria-label="${name}"]`);
  if (!el) throw new Error(`no control with aria-label=${name}`);
  return el as T;
}

function typeInto(el: HTMLInputElement | HTMLTextAreaElement, value: string): void {
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(proto, "value")?.set;
  if (!setter) throw new Error("no native value setter");
  act(() => {
    setter.call(el, value);
    el.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

function changeSelect(el: HTMLSelectElement, value: string): void {
  act(() => {
    el.value = value;
    el.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

function clickCheckbox(el: HTMLInputElement): void {
  act(() => {
    el.click();
  });
}

describe("FormFieldsOverlay", () => {
  describe("form-overlay-text", () => {
    it("renders kind text, not multiline, as input type=text with aria-label=name", () => {
      const markup = renderOverlay([field({ name: "FullName", kind: "text", multiline: false })]);
      const control = controlByAriaLabel(markup, "FullName");

      expect(control).toBeTruthy();
      expect(control!.startsWith("<input")).toBe(true);
      expect(getAttribute(control!, "type")).toBe("text");
      expect(getAttribute(control!, "aria-label")).toBe("FullName");
      expect(markup).not.toContain("<textarea");
    });
  });

  describe("form-overlay-multiline", () => {
    it("renders kind text + multiline as textarea, not input type=text", () => {
      const markup = renderOverlay([field({ name: "Notes", kind: "text", multiline: true })]);
      const control = controlByAriaLabel(markup, "Notes");

      expect(control).toBeTruthy();
      expect(control!.startsWith("<textarea")).toBe(true);
      expect(getAttribute(control!, "aria-label")).toBe("Notes");
      expect(markup).not.toMatch(/<input[^>]*type="text"/);
    });
  });

  describe("form-overlay-checkbox", () => {
    it("renders kind checkbox as input type=checkbox", () => {
      const markup = renderOverlay([field({ name: "Agree", kind: "checkbox" })]);
      const control = controlByAriaLabel(markup, "Agree");

      expect(control).toBeTruthy();
      expect(control!.startsWith("<input")).toBe(true);
      expect(getAttribute(control!, "type")).toBe("checkbox");
      expect(getAttribute(control!, "aria-label")).toBe("Agree");
    });

    it("is checked when the value is neither empty nor Off", () => {
      const fromField = renderOverlay([field({ name: "Agree", kind: "checkbox", value: "Yes" })]);
      const fromSession = renderOverlay(
        [field({ name: "Agree", kind: "checkbox", value: "Off" })],
        { Agree: "Agreed" },
      );

      expect(hasBooleanAttr(controlByAriaLabel(fromField, "Agree")!, "checked")).toBe(true);
      expect(hasBooleanAttr(controlByAriaLabel(fromSession, "Agree")!, "checked")).toBe(true);
    });

    it("is unchecked when the value is empty or Off", () => {
      const emptyField = renderOverlay([field({ name: "Agree", kind: "checkbox", value: "" })]);
      const offField = renderOverlay([field({ name: "Agree", kind: "checkbox", value: "Off" })]);
      const emptySession = renderOverlay(
        [field({ name: "Agree", kind: "checkbox", value: "Yes" })],
        { Agree: "" },
      );
      const offSession = renderOverlay(
        [field({ name: "Agree", kind: "checkbox", value: "Yes" })],
        { Agree: "Off" },
      );

      expect(hasBooleanAttr(controlByAriaLabel(emptyField, "Agree")!, "checked")).toBe(false);
      expect(hasBooleanAttr(controlByAriaLabel(offField, "Agree")!, "checked")).toBe(false);
      expect(hasBooleanAttr(controlByAriaLabel(emptySession, "Agree")!, "checked")).toBe(false);
      expect(hasBooleanAttr(controlByAriaLabel(offSession, "Agree")!, "checked")).toBe(false);
    });
  });

  describe("form-overlay-radio", () => {
    it("renders kind radio as a select of exportValues, not radio inputs", () => {
      const exportValues = ["Red", "Green", "Blue"];
      const markup = renderOverlay([
        field({ name: "Color", kind: "radio", exportValues, value: "Green" }),
      ]);
      const block = selectBlock(markup, "Color");

      expect(block).toBeTruthy();
      expect(getAttribute(block!.match(/^<select\b[^>]*>/)?.[0] ?? "", "aria-label")).toBe("Color");
      expect(optionValues(block!)).toEqual(exportValues);
      expect(markup).not.toMatch(/<input[^>]*type="radio"/);
    });
  });

  describe("form-overlay-combo", () => {
    it("renders kind combo with comboEdit false as a select of choices", () => {
      const choices = ["Apple", "Banana", "Cherry"];
      const markup = renderOverlay([
        field({ name: "Fruit", kind: "combo", comboEdit: false, choices, value: "Banana" }),
      ]);
      const block = selectBlock(markup, "Fruit");

      expect(block).toBeTruthy();
      expect(optionValues(block!)).toEqual(choices);
      expect(datalistBlock(markup)).toBeUndefined();
    });

    it("renders kind combo with comboEdit true as input plus datalist of choices", () => {
      const choices = ["Apple", "Banana", "Cherry"];
      const markup = renderOverlay([
        field({ name: "Fruit", kind: "combo", comboEdit: true, choices, value: "Banana" }),
      ]);
      const control = controlByAriaLabel(markup, "Fruit");
      const list = datalistBlock(markup);

      expect(control).toBeTruthy();
      expect(control!.startsWith("<input")).toBe(true);
      expect(getAttribute(control!, "aria-label")).toBe("Fruit");
      expect(list).toBeTruthy();
      expect(optionValues(list!)).toEqual(choices);
      expect(selectBlock(markup, "Fruit")).toBeUndefined();
    });
  });

  describe("form-overlay-list", () => {
    it("renders kind list as a select of choices", () => {
      const choices = ["One", "Two", "Three"];
      const markup = renderOverlay([
        field({ name: "Pick", kind: "list", choices, value: "Two" }),
      ]);
      const block = selectBlock(markup, "Pick");

      expect(block).toBeTruthy();
      expect(getAttribute(block!.match(/^<select\b[^>]*>/)?.[0] ?? "", "aria-label")).toBe("Pick");
      expect(optionValues(block!)).toEqual(choices);
    });
  });

  describe("form-overlay-existing-values", () => {
    it("lets values[name] win over field.value", () => {
      const markup = renderOverlay(
        [field({ name: "FullName", kind: "text", value: "Ada" })],
        { FullName: "Grace" },
      );
      const control = controlByAriaLabel(markup, "FullName");

      expect(getAttribute(control ?? "", "value")).toBe("Grace");
    });

    it("uses field.value when the values map omits the name", () => {
      const markup = renderOverlay([field({ name: "City", kind: "text", value: "Paris" })], {
        Other: "ignored",
      });
      const control = controlByAriaLabel(markup, "City");

      expect(getAttribute(control ?? "", "value")).toBe("Paris");
    });

    it("uses empty string when the map omits the name and field.value is null", () => {
      const textMarkup = renderOverlay([field({ name: "Empty", kind: "text", value: null })]);
      const areaMarkup = renderOverlay([
        field({ name: "BlankNotes", kind: "text", multiline: true, value: null }),
      ]);

      expect(getAttribute(controlByAriaLabel(textMarkup, "Empty") ?? "", "value")).toBe("");
      expect(textareaBody(areaMarkup, "BlankNotes")).toBe("");
    });
  });

  describe("form-overlay-readonly", () => {
    it("keeps a matching readOnly field present, disabled, with is-disabled wrapper", () => {
      const markup = renderOverlay([
        field({ name: "Locked", kind: "text", readOnly: true, value: "frozen" }),
      ]);
      const control = controlByAriaLabel(markup, "Locked");

      expect(control).toBeTruthy();
      expect(hasBooleanAttr(control!, "disabled")).toBe(true);
      expect(markup).toMatch(/\bis-disabled\b/);
      expect(markup).toContain("pdf-editor__form-chrome");
    });
  });

  describe("form-overlay-hidden", () => {
    it("shows a matching hidden field as present and disabled, not omitted", () => {
      const markup = renderOverlay([
        field({ name: "Secret", kind: "text", hidden: true, value: "hidden-val" }),
      ]);
      const control = controlByAriaLabel(markup, "Secret");

      expect(control).toBeTruthy();
      expect(hasBooleanAttr(control!, "disabled")).toBe(true);
      expect(markup).toMatch(/\bis-disabled\b/);
      expect(markup).toContain("pdf-editor__form-chrome");
    });
  });

  describe("form-overlay-page-filter", () => {
    it("shows only pageIndex === sourcePage-1; other page and null pageIndex are absent", () => {
      const fields = [
        field({ name: "OnPage", kind: "text", pageIndex: 1, value: "here" }),
        field({ name: "OtherPage", kind: "text", pageIndex: 0, value: "nope" }),
        field({ name: "NoPage", kind: "text", pageIndex: null, value: "nope" }),
      ];
      const markup = renderOverlay(fields, {}, 2);

      expect(controlByAriaLabel(markup, "OnPage")).toBeTruthy();
      expect(controlByAriaLabel(markup, "OtherPage")).toBeUndefined();
      expect(controlByAriaLabel(markup, "NoPage")).toBeUndefined();
      expect(markup).not.toContain('aria-label="OtherPage"');
      expect(markup).not.toContain('aria-label="NoPage"');
    });
  });

  describe("form-overlay-null-rect", () => {
    it("omits a matching-page field whose rect is null", () => {
      const markup = renderOverlay([
        field({ name: "HasRect", kind: "text", value: "ok" }),
        field({ name: "NoRect", kind: "text", rect: null, value: "gone" }),
      ]);

      expect(controlByAriaLabel(markup, "HasRect")).toBeTruthy();
      expect(controlByAriaLabel(markup, "NoRect")).toBeUndefined();
      expect(markup).not.toContain('aria-label="NoRect"');
      expect(markup).toContain("pdf-editor__form-chrome");
    });

    it("renders no pdf-editor__form-chrome when every field is unusable", () => {
      const markup = renderOverlay([
        field({ name: "NoRect", kind: "text", rect: null }),
        field({ name: "NoPage", kind: "text", pageIndex: null }),
        field({ name: "OtherPage", kind: "text", pageIndex: 1 }),
      ]);

      expect(markup).not.toContain("pdf-editor__form-chrome");
      expect(controlTags(markup)).toEqual([]);
    });
  });

  describe("form-overlay-onchange-text", () => {
    it("emits onChange(FullName, typed string) when typing into kind text", () => {
      const { container, onChange } = mountOverlay([
        field({ name: "FullName", kind: "text", multiline: false }),
      ]);
      const input = controlByAriaName<HTMLInputElement>(container, "FullName");

      typeInto(input, "Ada Lovelace");

      expect(onChange).toHaveBeenCalledTimes(1);
      expect(onChange).toHaveBeenCalledWith("FullName", "Ada Lovelace");
    });
  });

  describe("form-overlay-onchange-multiline", () => {
    it("emits onChange(Notes, typed string) when typing into a multiline textarea", () => {
      const { container, onChange } = mountOverlay([
        field({ name: "Notes", kind: "text", multiline: true }),
      ]);
      const area = controlByAriaName<HTMLTextAreaElement>(container, "Notes");

      typeInto(area, "line one\nline two");

      expect(onChange).toHaveBeenCalledTimes(1);
      expect(onChange).toHaveBeenCalledWith("Notes", "line one\nline two");
    });
  });

  describe("form-overlay-onchange-checkbox", () => {
    it("emits onChange(Agree, Yes) when clicking an unchecked box with empty exportValues", () => {
      const { container, onChange } = mountOverlay([
        field({ name: "Agree", kind: "checkbox", exportValues: [] }),
      ]);
      const box = controlByAriaName<HTMLInputElement>(container, "Agree");

      clickCheckbox(box);

      expect(onChange).toHaveBeenCalledTimes(1);
      expect(onChange).toHaveBeenCalledWith("Agree", "Yes");
    });

    it("emits onChange(Agree, Agreed) when exportValues[0] is Agreed", () => {
      const { container, onChange } = mountOverlay([
        field({ name: "Agree", kind: "checkbox", exportValues: ["Agreed"] }),
      ]);
      const box = controlByAriaName<HTMLInputElement>(container, "Agree");

      clickCheckbox(box);

      expect(onChange).toHaveBeenCalledTimes(1);
      expect(onChange).toHaveBeenCalledWith("Agree", "Agreed");
    });

    it("emits onChange(Agree, Off) when clicking a checked box", () => {
      const { container, onChange } = mountOverlay([
        field({ name: "Agree", kind: "checkbox", value: "Yes" }),
      ]);
      const box = controlByAriaName<HTMLInputElement>(container, "Agree");

      clickCheckbox(box);

      expect(onChange).toHaveBeenCalledTimes(1);
      expect(onChange).toHaveBeenCalledWith("Agree", "Off");
    });
  });

  describe("form-overlay-onchange-radio", () => {
    it("emits onChange(Color, chosen export value) when changing the radio select", () => {
      const exportValues = ["Red", "Green", "Blue"];
      const { container, onChange } = mountOverlay([
        field({ name: "Color", kind: "radio", exportValues, value: "Green" }),
      ]);
      const select = controlByAriaName<HTMLSelectElement>(container, "Color");

      changeSelect(select, "Blue");

      expect(onChange).toHaveBeenCalledTimes(1);
      expect(onChange).toHaveBeenCalledWith("Color", "Blue");
    });
  });

  describe("form-overlay-onchange-combo", () => {
    it("emits onChange(Fruit, choice) when changing a comboEdit false select", () => {
      const choices = ["Apple", "Banana", "Cherry"];
      const { container, onChange } = mountOverlay([
        field({ name: "Fruit", kind: "combo", comboEdit: false, choices, value: "Banana" }),
      ]);
      const select = controlByAriaName<HTMLSelectElement>(container, "Fruit");

      changeSelect(select, "Cherry");

      expect(onChange).toHaveBeenCalledTimes(1);
      expect(onChange).toHaveBeenCalledWith("Fruit", "Cherry");
    });

    it("emits onChange(Fruit, typed string) when typing into a comboEdit true input", () => {
      const choices = ["Apple", "Banana", "Cherry"];
      const { container, onChange } = mountOverlay([
        field({ name: "Fruit", kind: "combo", comboEdit: true, choices, value: "Banana" }),
      ]);
      const input = controlByAriaName<HTMLInputElement>(container, "Fruit");

      typeInto(input, "Mango");

      expect(onChange).toHaveBeenCalledTimes(1);
      expect(onChange).toHaveBeenCalledWith("Fruit", "Mango");
    });
  });

  describe("form-overlay-onchange-list", () => {
    it("emits onChange(Pick, choice) when changing the list select", () => {
      const choices = ["One", "Two", "Three"];
      const { container, onChange } = mountOverlay([
        field({ name: "Pick", kind: "list", choices, value: "Two" }),
      ]);
      const select = controlByAriaName<HTMLSelectElement>(container, "Pick");

      changeSelect(select, "Three");

      expect(onChange).toHaveBeenCalledTimes(1);
      expect(onChange).toHaveBeenCalledWith("Pick", "Three");
    });
  });

  describe("form-overlay-onchange-disabled", () => {
    it("does not call onChange when a readOnly checkbox is clicked", () => {
      const { container, onChange } = mountOverlay([
        field({ name: "Agree", kind: "checkbox", readOnly: true }),
      ]);
      const box = controlByAriaName<HTMLInputElement>(container, "Agree");

      clickCheckbox(box);

      expect(box.disabled).toBe(true);
      expect(onChange).not.toHaveBeenCalled();
    });

    it("does not call onChange when a hidden checkbox is clicked", () => {
      const { container, onChange } = mountOverlay([
        field({ name: "Agree", kind: "checkbox", hidden: true }),
      ]);
      const box = controlByAriaName<HTMLInputElement>(container, "Agree");

      clickCheckbox(box);

      expect(box.disabled).toBe(true);
      expect(onChange).not.toHaveBeenCalled();
    });
  });
});
