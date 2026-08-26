import { afterEach, describe, expect, it } from "vitest";
import { isTauriRuntime } from "./tauriEnv";

type MarkerKey = "__TAURI_INTERNALS__" | "__TAURI__" | "isTauri";

const MARKERS: MarkerKey[] = ["__TAURI_INTERNALS__", "__TAURI__", "isTauri"];

const initialWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
const baselineRuntime = isTauriRuntime();

function restoreWindow(): void {
  if (initialWindow) {
    Object.defineProperty(globalThis, "window", initialWindow);
    return;
  }
  Reflect.deleteProperty(globalThis, "window");
}

function installWindow(markers: Partial<Record<MarkerKey, unknown>> = {}): void {
  const next: Record<string, unknown> = {};
  for (const key of MARKERS) {
    if (Object.prototype.hasOwnProperty.call(markers, key)) {
      next[key] = markers[key];
    }
  }
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    enumerable: true,
    writable: true,
    value: next,
  });
}

function removeWindow(): void {
  Reflect.deleteProperty(globalThis, "window");
}

afterEach(() => {
  restoreWindow();
});

describe("isTauriRuntime", () => {
  it("isTauriRuntime_false_when_window_unavailable", () => {
    try {
      removeWindow();
      expect(isTauriRuntime()).toBe(false);
    } finally {
      restoreWindow();
    }
  });

  it("isTauriRuntime_false_when_browser_window_has_no_markers", () => {
    try {
      installWindow();
      expect(isTauriRuntime()).toBe(false);
    } finally {
      restoreWindow();
    }
  });

  it("isTauriRuntime_true_for___TAURI_INTERNALS___alone", () => {
    try {
      installWindow({ __TAURI_INTERNALS__: {} });
      expect(isTauriRuntime()).toBe(true);
    } finally {
      restoreWindow();
    }
  });

  it("isTauriRuntime_true_for___TAURI___alone", () => {
    try {
      installWindow({ __TAURI__: {} });
      expect(isTauriRuntime()).toBe(true);
    } finally {
      restoreWindow();
    }
  });

  it("isTauriRuntime_true_for_isTauri_alone", () => {
    try {
      installWindow({ isTauri: true });
      expect(isTauriRuntime()).toBe(true);
    } finally {
      restoreWindow();
    }
  });

  it("isTauriRuntime_falsey_markers_are_not_a_runtime", () => {
    try {
      const falseyValues: unknown[] = [false, 0, "", null, undefined];
      for (const value of falseyValues) {
        installWindow({
          __TAURI_INTERNALS__: value,
          __TAURI__: value,
          isTauri: value,
        });
        expect(isTauriRuntime()).toBe(false);
      }
      for (const key of MARKERS) {
        installWindow({ [key]: false } as Partial<Record<MarkerKey, unknown>>);
        expect(isTauriRuntime()).toBe(false);
      }
    } finally {
      restoreWindow();
    }
  });

  it("isTauriRuntime_restores_global_window_markers", () => {
    try {
      installWindow({ __TAURI_INTERNALS__: {}, __TAURI__: {}, isTauri: true });
      expect(isTauriRuntime()).toBe(true);
    } finally {
      restoreWindow();
    }
    expect(Object.getOwnPropertyDescriptor(globalThis, "window")).toEqual(
      initialWindow,
    );
    expect(isTauriRuntime()).toBe(baselineRuntime);
  });
});
