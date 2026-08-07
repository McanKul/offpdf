import { NavLink } from "react-router-dom";
import { Icon, type IconName } from "@/components/ui/Icon";
import { getTool } from "@/lib/tools";
import { ThemeToggle } from "./ThemeToggle";

const QUICK_TOOLS = [
  getTool("merge"),
  getTool("compress"),
  getTool("reorder"),
  getTool("images"),
];

function NavRow({ to, icon, label, end }: { to: string; icon: IconName; label: string; end?: boolean }) {
  return (
    <NavLink
      to={to}
      end={end}
      title={label}
      aria-label={label}
      className={({ isActive }) => `nav-item ${isActive ? "is-active" : ""}`}
    >
      <Icon name={icon} size={18} className="nav-item__icon" />
      <span className="sr-only">{label}</span>
    </NavLink>
  );
}

export function Sidebar() {
  return (
    <aside className="sidebar" aria-label="Primary navigation">
      <div className="sidebar__brand" title="OffPDF" aria-label="OffPDF">
        <div className="sidebar__logo">OP</div>
      </div>

      <nav className="sidebar__nav" aria-label="Workspace">
        <NavRow to="/" icon="home" label="All tools" end />
      </nav>

      <nav className="sidebar__nav" aria-label="Quick tools">
        {QUICK_TOOLS.map((tool) => (
          <NavRow key={tool.id} to={tool.path} icon={tool.icon} label={tool.name} />
        ))}
      </nav>

      <nav className="sidebar__nav sidebar__nav--footer" aria-label="App">
        <ThemeToggle />
        <NavRow to="/settings" icon="settings" label="Settings" />
        <NavRow to="/about" icon="info" label="About" />
      </nav>
    </aside>
  );
}
