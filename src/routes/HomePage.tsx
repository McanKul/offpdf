import { useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Card } from "@/components/ui/Card";
import { Badge } from "@/components/ui/Badge";
import { Button } from "@/components/ui/Button";
import { Icon } from "@/components/ui/Icon";
import { useToast } from "@/components/ui/Toast";
import { PrivacyBadge } from "@/components/layout/PrivacyBadge";
import { WorkspaceFilePicker } from "@/components/pdf";
import { TOOLS, CATEGORIES, getTool, type ToolCategory, type ToolMeta } from "@/lib/tools";
import { useJobsStore } from "@/state/jobsStore";
import { formatRelativeTime, dirname } from "@/lib/formatBytes";
import { openPath } from "@/lib/tauriCommands";
import { toAppError, type RecentJob } from "@/lib/types";

type CategoryFilter = ToolCategory | "All";

const FILTERS: CategoryFilter[] = ["All", ...CATEGORIES];
const FEATURED_TOOLS = [getTool("merge"), getTool("compress"), getTool("reorder")];
const FEATURED_TOOL_IDS = new Set(FEATURED_TOOLS.map((tool) => tool.id));
const FILTER_LABEL: Record<CategoryFilter, string> = {
  All: "All",
  Organize: "Organize",
  Convert: "Convert",
  "Optimize & secure": "Secure",
};
const CATEGORY_CLASS: Record<ToolCategory, string> = {
  Organize: "organize",
  Convert: "convert",
  "Optimize & secure": "optimize",
};

function categoryClass(tool: ToolMeta) {
  return CATEGORY_CLASS[tool.category];
}

function StatusBadge({ status }: { status: RecentJob["status"] }) {
  if (status === "completed") return <Badge variant="success">Completed</Badge>;
  if (status === "failed") return <Badge variant="danger">Failed</Badge>;
  return <Badge variant="warning">Cancelled</Badge>;
}

function ToolButton({ tool, onOpen }: { tool: ToolMeta; onOpen: (path: string) => void }) {
  return (
    <button
      type="button"
      className={`tool-card tool-card--${categoryClass(tool)}`}
      onClick={() => onOpen(tool.path)}
      title={tool.description}
    >
      <span className="tool-card__icon">
        <Icon name={tool.icon} size={22} />
      </span>
      <span className="tool-card__text">
        <span className="tool-card__name">{tool.name}</span>
        <span className="tool-card__desc">{tool.description}</span>
      </span>
    </button>
  );
}

function FeaturedTool({ tool, onOpen }: { tool: ToolMeta; onOpen: (path: string) => void }) {
  return (
    <button
      type="button"
      className={`featured-tool featured-tool--${categoryClass(tool)}`}
      onClick={() => onOpen(tool.path)}
      title={tool.description}
    >
      <span className="featured-tool__icon">
        <Icon name={tool.icon} size={24} />
      </span>
      <span className="featured-tool__text">
        <span className="featured-tool__name">{tool.name}</span>
        <span className="featured-tool__desc">{tool.description}</span>
      </span>
      <Icon name="arrowRight" size={16} className="featured-tool__go" />
    </button>
  );
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
  const [activeFilter, setActiveFilter] = useState<CategoryFilter>("All");

  const categoryCounts = useMemo(() => {
    const counts: Record<ToolCategory, number> = {
      Organize: 0,
      Convert: 0,
      "Optimize & secure": 0,
    };
    for (const tool of TOOLS) {
      counts[tool.category] += 1;
    }
    return counts;
  }, []);

  const filteredTools = useMemo(() => {
    return TOOLS.filter((tool) => {
      const inCategory = activeFilter === "All" || tool.category === activeFilter;
      const isFeaturedDefault = activeFilter === "All" && FEATURED_TOOL_IDS.has(tool.id);
      return inCategory && !isFeaturedDefault;
    });
  }, [activeFilter]);

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

      <section className="home-tools">
        <div className="section-label">Tools</div>

        <div className="category-pills" role="list" aria-label="Tool categories">
          {FILTERS.map((filter) => {
            const count = filter === "All" ? TOOLS.length : categoryCounts[filter];
            return (
              <button
                key={filter}
                type="button"
                className={`category-pill ${activeFilter === filter ? "is-active" : ""}`}
                onClick={() => setActiveFilter(filter)}
                aria-pressed={activeFilter === filter}
              >
                <span>{FILTER_LABEL[filter]}</span>
                <span>{count}</span>
              </button>
            );
          })}
        </div>

        {activeFilter === "All" && (
          <div className="featured-strip" aria-label="Quick start tools">
            {FEATURED_TOOLS.map((tool) => (
              <FeaturedTool key={tool.id} tool={tool} onOpen={openTool} />
            ))}
          </div>
        )}

        <div className="tool-grid">
          {filteredTools.map((tool) => (
            <ToolButton key={tool.id} tool={tool} onOpen={openTool} />
          ))}
        </div>
      </section>

      <RecentJobs />
    </div>
  );
}
