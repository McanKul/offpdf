import type { ReactNode } from "react";

/** Collapsible "Technical details" block used by the error panel. */
export function Details({
  summary = "Technical details",
  children,
}: {
  summary?: string;
  children: ReactNode;
}) {
  return (
    <details className="details">
      <summary>{summary}</summary>
      <div className="details__content selectable">{children}</div>
    </details>
  );
}
