import { useSettingsStore, type Theme } from "@/state/settingsStore";
import { Icon, type IconName } from "@/components/ui/Icon";

const ORDER: Theme[] = ["light", "dark", "system"];
const LABEL: Record<Theme, string> = { light: "Light", dark: "Dark", system: "System" };
const ICON: Record<Theme, IconName> = { light: "sun", dark: "moon", system: "monitor" };

/** Cycles light → dark → system. */
export function ThemeToggle() {
  const theme = useSettingsStore((s) => s.theme);
  const setTheme = useSettingsStore((s) => s.setTheme);

  const next = () => {
    const idx = ORDER.indexOf(theme);
    setTheme(ORDER[(idx + 1) % ORDER.length]);
  };

  return (
    <button
      className="btn btn--ghost btn--sm"
      onClick={next}
      title={`Theme: ${LABEL[theme]} (click to change)`}
      aria-label={`Theme: ${LABEL[theme]}`}
    >
      <Icon name={ICON[theme]} size={16} />
      <span className="sr-only">{LABEL[theme]}</span>
    </button>
  );
}
