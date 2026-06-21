import type { ReactNode } from "react";
import { Link } from "react-router-dom";
import { Icon } from "@/components/ui/Icon";
import type { ToolMeta } from "@/lib/tools";

/** Shared header + container for every tool page. */
export function ToolPage({ tool, children }: { tool: ToolMeta; children: ReactNode }) {
  return (
    <div className="tool-page">
      <div className="page-header">
        <Link to="/" className="page-header__back">
          <Icon name="home" size={14} />
          All tools
        </Link>
        <div className="page-header__top">
          <div className="page-header__icon">
            <Icon name={tool.icon} size={22} />
          </div>
          <h1 className="page-header__title">{tool.name}</h1>
        </div>
        <p className="page-header__desc">{tool.longDescription}</p>
      </div>
      <div className="tool-layout">{children}</div>
    </div>
  );
}

/** A labelled card section used inside tool pages. */
export function ToolSection({
  label,
  sublabel,
  children,
}: {
  label: string;
  sublabel?: string;
  children: ReactNode;
}) {
  return (
    <div className="card card--pad">
      <div style={{ marginBottom: 12 }}>
        <div className="section-label">{label}</div>
        {sublabel && <div className="section-sub">{sublabel}</div>}
      </div>
      {children}
    </div>
  );
}
