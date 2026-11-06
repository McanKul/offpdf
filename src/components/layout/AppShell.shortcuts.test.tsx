// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { HashRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ToastProvider } from "@/components/ui/Toast";
import { AppShell } from "./AppShell";

let container: HTMLDivElement;
let root: Root | null = null;

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  act(() => {
    root = createRoot(container);
    root.render(
      <ToastProvider>
        <HashRouter>
          <AppShell />
        </HashRouter>
      </ToastProvider>,
    );
  });
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  root = null;
  container.remove();
});

function pressOn(target: EventTarget, key: string) {
  act(() => {
    target.dispatchEvent(
      new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }),
    );
  });
}

describe("AppShell keyboard shortcuts", () => {
  it("SHORTCUTS-OPEN: `?` opens the reference and Escape closes it", () => {
    expect(container.querySelector('[role="dialog"]')).toBeNull();

    pressOn(document.body, "?");

    const dialog = container.querySelector<HTMLElement>('[role="dialog"]');
    expect(dialog).not.toBeNull();
    expect(dialog?.textContent).toContain("Keyboard shortcuts");

    pressOn(dialog!, "Escape");
    expect(container.querySelector('[role="dialog"]')).toBeNull();
  });

  it("SHORTCUTS-GUARD: `?` typed in an editable field does not open the reference", () => {
    const input = document.createElement("input");
    document.body.appendChild(input);
    input.focus();

    pressOn(input, "?");

    expect(container.querySelector('[role="dialog"]')).toBeNull();
    input.remove();
  });

  it("SHORTCUTS-MODAL: `?` does not open over another dialog", () => {
    const existingDialog = document.createElement("div");
    existingDialog.setAttribute("role", "dialog");
    existingDialog.setAttribute("aria-modal", "true");
    document.body.appendChild(existingDialog);

    pressOn(existingDialog, "?");

    expect(container.querySelector('[role="dialog"]')).toBeNull();
    existingDialog.remove();
  });
});
