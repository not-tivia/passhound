import { useEffect, useState } from "react";
import { api } from "../api";
import type { TagWithCount, GuiError } from "../types";

interface TagsSidebarProps {
  filterTagIds: number[];
  onFilterChange: (ids: number[]) => void;
  onManageClick: () => void;
  onMergeClick: () => void;
  onLockedError: () => void;
  refreshKey: number;
}

export default function TagsSidebar({ filterTagIds, onFilterChange, onManageClick, onMergeClick, onLockedError, refreshKey }: TagsSidebarProps) {
  const [tags, setTags] = useState<TagWithCount[]>([]);

  useEffect(() => {
    api.listTags().then(setTags, (e) => {
      if ((e as GuiError).kind === "Locked") onLockedError();
    });
  }, [onLockedError, refreshKey]);

  const toggle = (id: number, ctrl: boolean) => {
    if (ctrl) {
      onFilterChange(
        filterTagIds.includes(id)
          ? filterTagIds.filter((x) => x !== id)
          : [...filterTagIds, id]
      );
    } else {
      // Single-select: replace, or clear if clicking the already-only-active tag
      onFilterChange(filterTagIds.includes(id) && filterTagIds.length === 1 ? [] : [id]);
    }
  };

  return (
    <aside className="tags-sidebar">
      <header className="tags-sidebar__header">
        <span className="tags-sidebar__title">Tags</span>
        <button className="tags-sidebar__manage" onClick={onMergeClick} title="Merge duplicate sites">&#9112;</button>
        <button className="tags-sidebar__manage" onClick={onManageClick} title="Manage tags">&#9881;</button>
      </header>
      {filterTagIds.length > 0 && (
        <button className="tags-sidebar__clear" onClick={() => onFilterChange([])}>
          Clear filters
        </button>
      )}
      {tags.length === 0 ? (
        <div className="tags-sidebar__empty">No tags yet. Select accounts and click &ldquo;Add tag&rdquo; to create one.</div>
      ) : (
        <ul className="tags-sidebar__list">
          {tags.map((t) => {
            const active = filterTagIds.includes(t.id);
            return (
              <li
                key={t.id}
                className={`tags-sidebar__row${active ? " tags-sidebar__row--active" : ""}`}
                onClick={(e) => toggle(t.id, e.ctrlKey || e.metaKey)}
              >
                <span className="tags-sidebar__check">{active ? "\u2713" : ""}</span>
                <span className="tags-sidebar__name">{t.name}</span>
                <span className="tags-sidebar__count">{t.account_count}</span>
              </li>
            );
          })}
        </ul>
      )}
    </aside>
  );
}
