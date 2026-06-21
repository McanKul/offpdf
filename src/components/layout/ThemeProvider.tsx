import { useEffect, type ReactNode } from "react";
import { useSettingsStore, resolveTheme } from "@/state/settingsStore";

/** Applies the resolved theme to <html data-theme> and reacts to OS changes. */
export function ThemeProvider({ children }: { children: ReactNode }) {
  const theme = useSettingsStore((s) => s.theme);

  useEffect(() => {
    const apply = () => {
      document.documentElement.setAttribute("data-theme", resolveTheme(theme));
    };
    apply();

    if (theme === "system" && window.matchMedia) {
      const mql = window.matchMedia("(prefers-color-scheme: dark)");
      mql.addEventListener("change", apply);
      return () => mql.removeEventListener("change", apply);
    }
  }, [theme]);

  return <>{children}</>;
}
