import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { SiteSummary, GuiError } from "../types";

interface SitePickerProps {
  suggestions?: SiteSummary[];
  placeholder?: string;
  onSelect: (site: SiteSummary | { name: string; isNew: true }) => void;
  onCancel: () => void;
  onLockedError: () => void;
}

export default function SitePicker({ suggestions, placeholder, onSelect, onCancel, onLockedError }: SitePickerProps) {
  const [query, setQuery] = useState("");
  const [allSites, setAllSites] = useState<SiteSummary[]>([]);
  const [highlight, setHighlight] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
    if (!suggestions) {
      api.listSites().then(
        (ss) => setAllSites(ss.map(({ id, name }) => ({ id, name }))),
        (e) => { if ((e as GuiError).kind === "Locked") onLockedError(); }
      );
    }
  }, [onLockedError, suggestions]);

  const pool = suggestions ?? allSites;
  const filtered = pool.filter((s) => s.name.toLowerCase().includes(query.toLowerCase()));
  const exact = pool.find((s) => s.name.toLowerCase() === query.toLowerCase());

  const submit = () => {
    if (filtered[highlight]) onSelect(filtered[highlight]);
    else if (query.trim() && !exact) onSelect({ name: query.trim(), isNew: true });
  };

  return (
    <div className="tag-picker">
      <input
        ref={inputRef}
        value={query}
        onChange={(e) => { setQuery(e.target.value); setHighlight(0); }}
        onKeyDown={(e) => {
          if (e.key === "Enter") { e.preventDefault(); submit(); }
          else if (e.key === "Escape") { e.preventDefault(); onCancel(); }
          else if (e.key === "ArrowDown") { e.preventDefault(); setHighlight((h) => Math.min(h + 1, filtered.length - 1)); }
          else if (e.key === "ArrowUp") { e.preventDefault(); setHighlight((h) => Math.max(h - 1, 0)); }
        }}
        placeholder={placeholder ?? "site name"}
        className="tag-picker__input"
      />
      <ul className="tag-picker__list" role="listbox">
        {filtered.map((s, i) => (
          <li
            key={s.id}
            className={`tag-picker__option${i === highlight ? " tag-picker__option--highlight" : ""}`}
            onMouseEnter={() => setHighlight(i)}
            onMouseDown={(e) => { e.preventDefault(); onSelect(s); }}
            role="option"
            aria-selected={i === highlight}
          >
            {s.name}
          </li>
        ))}
        {query.trim() && !exact && (
          <li
            className="tag-picker__option tag-picker__option--create"
            onMouseDown={(e) => { e.preventDefault(); onSelect({ name: query.trim(), isNew: true }); }}
          >
            Create &ldquo;{query.trim()}&rdquo;
          </li>
        )}
      </ul>
    </div>
  );
}
