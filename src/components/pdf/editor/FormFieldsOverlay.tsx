/**
 * HTML form chrome at lopdf widget rects. Not overlay stamps; not pdf.js annotations.
 */
import { makeMapping, pdfRectToViewport, type FormField } from "@/lib/editor";
import type { PageLayout } from "./PageSurface";

export function FormFieldsOverlay({
  layout,
  fields,
  values,
  sourcePage,
  onChange,
}: {
  layout: PageLayout;
  fields: FormField[];
  values: Record<string, string>;
  /** 1-based page number inside the source file. */
  sourcePage: number;
  onChange: (name: string, value: string) => void;
}) {
  const mapping = makeMapping(layout.geometry, layout.cssWidth, layout.cssHeight);
  const pageFields = fields.filter(
    (f) => f.rect && f.pageIndex === sourcePage - 1,
  );
  if (pageFields.length === 0) return null;

  return (
    <div className="pdf-editor__form-chrome" aria-label="Form fields">
      {pageFields.map((field) => {
        const css = pdfRectToViewport(field.rect!, mapping);
        const value = values[field.name] ?? field.value ?? "";
        const disabled = field.readOnly || field.hidden;
        return (
          <div
            key={field.name}
            className={`pdf-editor__form-field${disabled ? " is-disabled" : ""}`}
            style={{
              left: css.x,
              top: css.y,
              width: Math.max(css.w, 12),
              height: Math.max(css.h, 12),
            }}
          >
            <FieldControl field={field} value={value} disabled={disabled} onChange={onChange} />
          </div>
        );
      })}
    </div>
  );
}

function FieldControl({
  field,
  value,
  disabled,
  onChange,
}: {
  field: FormField;
  value: string;
  disabled: boolean;
  onChange: (name: string, value: string) => void;
}) {
  const name = field.name;
  if (field.kind === "checkbox") {
    const on = field.exportValues[0] ?? "Yes";
    const checked = value !== "" && value !== "Off";
    return (
      <input
        type="checkbox"
        className="pdf-editor__form-input"
        aria-label={name}
        disabled={disabled}
        checked={checked}
        onChange={(e) => onChange(name, e.target.checked ? on : "Off")}
      />
    );
  }
  if (field.kind === "radio") {
    return (
      <select
        className="pdf-editor__form-input"
        aria-label={name}
        disabled={disabled}
        value={value}
        onChange={(e) => onChange(name, e.target.value)}
      >
        {field.exportValues.map((opt) => (
          <option key={opt} value={opt}>
            {opt}
          </option>
        ))}
      </select>
    );
  }
  if (field.kind === "combo" || field.kind === "list") {
    if (field.kind === "combo" && field.comboEdit) {
      const listId = `offpdf-form-opt-${name.replace(/[^A-Za-z0-9_-]/g, "_")}`;
      return (
        <>
          <input
            className="pdf-editor__form-input"
            aria-label={name}
            disabled={disabled}
            maxLength={field.maxLen ?? undefined}
            list={listId}
            value={value}
            onChange={(e) => onChange(name, e.target.value)}
          />
          <datalist id={listId}>
            {field.choices.map((opt) => (
              <option key={opt} value={opt} />
            ))}
          </datalist>
        </>
      );
    }
    return (
      <select
        className="pdf-editor__form-input"
        aria-label={name}
        disabled={disabled}
        value={value}
        onChange={(e) => onChange(name, e.target.value)}
      >
        {field.choices.map((opt) => (
          <option key={opt} value={opt}>
            {opt}
          </option>
        ))}
      </select>
    );
  }
  if (field.multiline) {
    return (
      <textarea
        className="pdf-editor__form-input"
        aria-label={name}
        disabled={disabled}
        maxLength={field.maxLen ?? undefined}
        value={value}
        onChange={(e) => onChange(name, e.target.value)}
      />
    );
  }
  return (
    <input
      type="text"
      className="pdf-editor__form-input"
      aria-label={name}
      disabled={disabled}
      maxLength={field.maxLen ?? undefined}
      value={value}
      onChange={(e) => onChange(name, e.target.value)}
    />
  );
}
