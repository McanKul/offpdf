import { useEffect, type ReactNode } from "react";
import { parsePageRange } from "@/lib/pageRange";

/**
 * Page-range text input with live validation against `parsePageRange`.
 * `mode="order"` preserves the typed order and allows duplicates (reorder).
 */
export function PageRangeInput({
  value,
  onChange,
  pageCount,
  mode = "set",
  allowAll = true,
  label,
  hint,
  placeholder = "e.g. 1,3,5-8",
  onValidChange,
}: {
  value: string;
  onChange: (value: string) => void;
  pageCount?: number;
  mode?: "set" | "order";
  allowAll?: boolean;
  label?: ReactNode;
  hint?: ReactNode;
  placeholder?: string;
  onValidChange?: (valid: boolean) => void;
}) {
  const trimmed = value.trim();
  const result =
    trimmed.length === 0
      ? null
      : parsePageRange(value, { pageCount, preserveOrder: mode === "order", allowAll });

  const valid = result?.ok === true;
  const error = result && !result.ok ? result.error : null;

  useEffect(() => {
    onValidChange?.(valid);
  }, [valid, onValidChange]);

  return (
    <div className="field">
      {label && <label className="field__label">{label}</label>}
      <input
        className={`input ${error ? "is-invalid" : ""}`}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        spellCheck={false}
        autoCapitalize="off"
        autoCorrect="off"
      />
      {error ? (
        <span className="field__error">{error}</span>
      ) : hint ? (
        <span className="field__hint">{hint}</span>
      ) : null}
    </div>
  );
}
