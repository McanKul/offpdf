import { useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { HomeToolsSection } from "@/components/home/HomeToolsSection";
import { Card } from "@/components/ui/Card";
import { Badge } from "@/components/ui/Badge";
import { Button } from "@/components/ui/Button";
import { Icon } from "@/components/ui/Icon";
import { useToast } from "@/components/ui/Toast";
import { PrivacyBadge } from "@/components/layout/PrivacyBadge";
import { WorkspaceFilePicker } from "@/components/pdf";
import { isHomeSearchFocusHotkey } from "@/lib/homeSearchHotkey";
import { TOOLS, getTool, type ToolCategory } from "@/lib/tools";
import { useJobsStore } from "@/state/jobsStore";
import { formatRelativeTime, dirname } from "@/lib/formatBytes";
import { openPath } from "@/lib/tauriCommands";
import { toAppError, type RecentJob } from "@/lib/types";

function StatusBadge({ status }: { status: RecentJob["status"] }) {
  if (status === "completed") return <Badge variant="success">Completed</Badge>;
  if (status === "failed") return <Badge variant="danger">Failed</Badge>;
  return <Badge variant="warning">Cancelled</Badge>;
}

/** Compact, full-width recent-activity strip. Renders nothing when empty. */
function RecentJobs() {
  const recentJobs = useJobsStore((s) => s.recentJobs);
  const clearJobs = useJobsStore((s) => s.clearJobs);
  const { toast } = useToast();

  const open = async (path: string) => {
    try {
      await openPath(path);
    } catch (e) {
      toast({ title: "Could not open", description: toAppError(e).message, variant: "error" });
    }
  };

  if (recentJobs.length === 0) return null;

  return (
    <section className="home-recent">
      <div className="spread" style={{ marginBottom: 8 }}>
        <div className="section-label">Recent activity</div>
        <Button variant="ghost" size="sm" onClick={clearJobs}>
          Clear
        </Button>
      </div>
      <div className="recent-grid">
        {recentJobs.slice(0, 6).map((job) => {
          const tool = getTool(job.tool);
          const folder = job.outputPaths[0] ? dirname(job.outputPaths[0]) : "";
          return (
            <Card padded key={job.id} className="recent-card">
              <div className="file-row">
                <div className="file-row__icon" style={{ background: "var(--primary-soft)", color: "var(--primary)" }}>
                  <Icon name={tool.icon} size={18} />
                </div>
                <div className="grow">
                  <div className="file-row__name truncate">{job.label}</div>
                  <div className="file-row__meta">
                    <StatusBadge status={job.status} />
                    <span>· {formatRelativeTime(job.finishedAt)}</span>
                  </div>
                </div>
                {job.outputPaths[0] && (
                  <Button variant="ghost" size="sm" onClick={() => open(job.outputPaths[0])} title="Open file">
                    <Icon name="external" size={16} />
                  </Button>
                )}
                {folder && (
                  <Button variant="ghost" size="sm" onClick={() => open(folder)} title="Open folder">
                    <Icon name="folderOpen" size={16} />
                  </Button>
                )}
              </div>
            </Card>
          );
        })}
      </div>
    </section>
  );
}

export function HomePage() {
  const navigate = useNavigate();
  const searchInputRef = useRef<HTMLInputElement>(null);
  const [query, setQuery] = useState("");
  const [activeFilter, setActiveFilter] = useState<ToolCategory | "All">("All");

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!isHomeSearchFocusHotkey(event)) return;
      event.preventDefault();
      searchInputRef.current?.focus();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const openTool = (path: string) => navigate(path);

  return (
    <div className="home-page">
      <section className="home-hero">
        <div className="home-hero__copy">
          <div className="home-hero__logo">OP</div>
          <div>
            <h1>OffPDF</h1>
            <p className="home-hero__tagline">
              Work with PDFs, privately — your files never leave your computer.
            </p>
          </div>
        </div>
        <div className="home-hero__badges">
          <PrivacyBadge compact />
          <Badge variant="neutral">{TOOLS.length} tools</Badge>
          <Badge variant="neutral">Offline</Badge>
        </div>
      </section>

      <section className="home-intake" aria-label="Add files">
        <WorkspaceFilePicker selectable={false} />
      </section>

      <HomeToolsSection
        query={query}
        onQueryChange={setQuery}
        category={activeFilter}
        onCategory={setActiveFilter}
        onOpen={openTool}
        inputRef={searchInputRef}
      />

      <RecentJobs />
    </div>
  );
}
