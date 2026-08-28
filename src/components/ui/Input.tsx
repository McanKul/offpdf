import { useId, type InputHTMLAttributes, type ReactNode } from "react";

export interface InputProps extends InputHTMLAttributes<HTMLInputElement> {
  label?: ReactNode;
  hint?: ReactNode;
  error?: string | null;
}

export function Input({ label, hint, error, className, id, ...rest }: InputProps) {
  const generatedId = useId();
  const inputId = id ?? rest.name ?? `input-${generatedId.replace(/:/g, "")}`;
  return (
    <div className="field">
      {label && (
        <label className="field__label" htmlFor={inputId}>
          {label}
        </label>
      )}
      <input
        id={inputId}
        className={["input", error ? "is-invalid" : "", className ?? ""].filter(Boolean).join(" ")}
        {...rest}
      />
      {error ? (
        <span className="field__error">{error}</span>
      ) : hint ? (
        <span className="field__hint">{hint}</span>
      ) : null}
    </div>
  );
}
