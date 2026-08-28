import { useId, type SelectHTMLAttributes, type ReactNode } from "react";

export interface SelectOption {
  value: string;
  label: string;
}

export interface SelectProps
  extends Omit<SelectHTMLAttributes<HTMLSelectElement>, "onChange"> {
  label?: ReactNode;
  hint?: ReactNode;
  options: SelectOption[];
  value: string;
  onChange: (value: string) => void;
}

export function Select({ label, hint, options, value, onChange, id, name, ...rest }: SelectProps) {
  const generatedId = useId();
  const selectId = id ?? name ?? `select-${generatedId.replace(/:/g, "")}`;
  return (
    <div className="field">
      {label && (
        <label className="field__label" htmlFor={selectId}>
          {label}
        </label>
      )}
      <select
        id={selectId}
        name={name}
        className="select"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        {...rest}
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
      {hint && <span className="field__hint">{hint}</span>}
    </div>
  );
}
