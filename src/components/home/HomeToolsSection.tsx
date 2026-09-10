import { useRef, type KeyboardEvent, type MutableRefObject, type Ref } from "react";
import { Icon } from "@/components/ui/Icon";
import { homeSearchEscapeAction } from "@/lib/homeSearchEscape";
import { homeToolsView } from "@/lib/homeToolsView";
import { CATEGORIES, TOOLS, type ToolCategory, type ToolMeta } from "@/lib/tools";

type CategoryFilter = ToolCategory | "All";

const FILTERS: CategoryFilter[] = ["All", ...CATEGORIES];
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

const CATEGORY_COUNTS: Record<ToolCategory, number> = {
  Organize: 0,
  Convert: 0,
  "Optimize & secure": 0,
};
for (const tool of TOOLS) {
  CATEGORY_COUNTS[tool.category] += 1;
}

function categoryClass(tool: ToolMeta) {
  return CATEGORY_CLASS[tool.category];
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

export function HomeToolsSection({
  query,
  onQueryChange,
  category,
  onCategory,
  onOpen,
  inputRef,
}: {
  query: string;
  onQueryChange: (query: string) => void;
  category: CategoryFilter;
  onCategory: (category: CategoryFilter) => void;
  onOpen: (path: string) => void;
  inputRef?: Ref<HTMLInputElement>;
}) {
  const view = homeToolsView({ tools: TOOLS, query, category });
  const localInputRef = useRef<HTMLInputElement | null>(null);

  const setSearchInputRef = (node: HTMLInputElement | null) => {
    localInputRef.current = node;
    if (typeof inputRef === "function") inputRef(node);
    else if (inputRef) (inputRef as MutableRefObject<HTMLInputElement | null>).current = node;
  };

  const focusSearchInput = () => {
    const fromProp = inputRef && typeof inputRef !== "function" ? inputRef.current : null;
    (fromProp ?? localInputRef.current)?.focus();
  };

  const clearSearch = () => {
    onQueryChange("");
    focusSearchInput();
  };

  const onSearchKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    const action = homeSearchEscapeAction(event);
    if (!action.clear && !action.blur) return;
    if (action.clear) onQueryChange("");
    if (action.blur) event.currentTarget.blur();
  };

  return (
    <section className="home-tools">
      <div className="section-label">Tools</div>

      <div className="tool-search">
        <Icon name="search" size={16} />
        <input
          ref={setSearchInputRef}
          type="search"
          value={query}
          onChange={(event) => onQueryChange(event.target.value)}
          onKeyDown={onSearchKeyDown}
          aria-label="Search tools"
          placeholder="Search tools"
          spellCheck={false}
          autoComplete="off"
        />
        {query ? (
          <button
            type="button"
            className="tool-search__clear"
            aria-label="Clear search"
            onMouseDown={(event) => event.preventDefault()}
            onClick={clearSearch}
          >
            <Icon name="x" size={15} />
          </button>
        ) : null}
      </div>

      <div className="category-pills" role="list" aria-label="Tool categories">
        {FILTERS.map((filter) => {
          const count = filter === "All" ? TOOLS.length : CATEGORY_COUNTS[filter];
          return (
            <button
              key={filter}
              type="button"
              className={`category-pill ${category === filter ? "is-active" : ""}`}
              onClick={() => onCategory(filter)}
              aria-pressed={category === filter}
            >
              <span>{FILTER_LABEL[filter]}</span>
              <span>{count}</span>
            </button>
          );
        })}
      </div>

      {view.featured.length > 0 && (
        <div className="featured-strip" aria-label="Quick start tools">
          {view.featured.map((tool) => (
            <FeaturedTool key={tool.id} tool={tool} onOpen={onOpen} />
          ))}
        </div>
      )}

      {view.noResults ? (
        <p className="no-results">No tools match your search.</p>
      ) : (
        <div className="tool-grid">
          {view.grid.map((tool) => (
            <ToolButton key={tool.id} tool={tool} onOpen={onOpen} />
          ))}
        </div>
      )}
    </section>
  );
}
