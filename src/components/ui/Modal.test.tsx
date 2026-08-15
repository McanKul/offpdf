import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { Modal } from "./Modal";

const noop = () => {};

function getAttribute(tag: string, name: string) {
  return tag.match(new RegExp(`\\s${name}="([^"]+)"`))?.[1];
}

function getDialogTags(markup: string) {
  return markup.match(/<div[^>]*\srole="dialog"[^>]*>/g) ?? [];
}

function getTitleElements(markup: string) {
  return [...markup.matchAll(/<div class="modal__title"([^>]*)>(.*?)<\/div>/g)].map(
    ([, attributes, content]) => ({ attributes, content }),
  );
}

function getOnly<T>(items: T[], description: string): T {
  if (items.length !== 1) throw new Error(`Expected one ${description}, found ${items.length}`);
  return items[0]!;
}

describe("Modal accessibility", () => {
  it("associates a meaningful plain-text title with the dialog", () => {
    const markup = renderToStaticMarkup(
      <Modal open onClose={noop} title="Delete document?">
        This action cannot be undone.
      </Modal>,
    );
    const dialog = getOnly([...getDialogTags(markup)], "dialog");
    const title = getOnly(getTitleElements(markup), "modal title");

    expect(title.content).toBe("Delete document?");
    expect(getAttribute(title.attributes, "id")).toBeTruthy();
    expect(getAttribute(dialog, "aria-labelledby")).toBe(getAttribute(title.attributes, "id"));
  });

  it("associates a nested JSX title with the dialog", () => {
    const markup = renderToStaticMarkup(
      <Modal
        open
        onClose={noop}
        title={
          <span>
            Export <strong>PDF</strong>
          </span>
        }
      >
        Choose export settings.
      </Modal>,
    );
    const dialog = getOnly([...getDialogTags(markup)], "dialog");
    const title = getOnly(getTitleElements(markup), "modal title");

    expect(title.content).toBe("<span>Export <strong>PDF</strong></span>");
    expect(getAttribute(title.attributes, "id")).toBeTruthy();
    expect(getAttribute(dialog, "aria-labelledby")).toBe(getAttribute(title.attributes, "id"));
  });

  it("uses unique title IDs for sibling modals", () => {
    const markup = renderToStaticMarkup(
      <>
        <Modal open onClose={noop} title="First modal">
          First body
        </Modal>
        <Modal open onClose={noop} title="Second modal">
          Second body
        </Modal>
      </>,
    );
    const dialogs = getDialogTags(markup);
    const titles = getTitleElements(markup);
    const titleIds = titles.map(({ attributes }) => getAttribute(attributes, "id"));

    expect(dialogs).toHaveLength(2);
    expect(titles).toHaveLength(2);
    expect(titleIds.every(Boolean)).toBe(true);
    expect(new Set(titleIds).size).toBe(2);
    expect(dialogs.map((dialog) => getAttribute(dialog, "aria-labelledby"))).toEqual(titleIds);
  });
});
