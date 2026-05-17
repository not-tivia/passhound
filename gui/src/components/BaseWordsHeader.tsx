import type { BaseWordsSortMode } from "../preferences";

interface BaseWordsHeaderProps {
  total: number;
  revealAll: boolean;
  analyzing: boolean;
  onToggleReveal: () => void;
  onReanalyze: () => void;
  onAddClick: () => void;
  sortMode: BaseWordsSortMode;
  onSortChange: (mode: BaseWordsSortMode) => void;
}

export default function BaseWordsHeader({
  total,
  revealAll,
  analyzing,
  onToggleReveal,
  onReanalyze,
  onAddClick,
  sortMode,
  onSortChange,
}: BaseWordsHeaderProps) {
  return (
    <div className="base-words-header">
      <span className="base-words-header__title">Base Words</span>
      <span className="base-words-header__count">{total} total</span>
      <button onClick={onAddClick}>+ Add</button>
      <select
        value={sortMode}
        onChange={(e) => onSortChange(e.target.value as BaseWordsSortMode)}
        className="base-words__sort-select"
      >
        <option value="usage">Sort: usage ↓</option>
        <option value="alpha">Sort: A → Z</option>
        <option value="last_seen">Sort: last seen ↓</option>
      </select>
      <button
        className="base-words-header__reanalyze"
        onClick={onReanalyze}
        disabled={analyzing}
      >
        {analyzing ? "Analyzing…" : "\u{21BB} Re-analyze"}
      </button>
      <button
        className="base-words-header__reveal"
        onClick={onToggleReveal}
        disabled={total === 0}
      >
        {revealAll ? "Hide words" : "\u{1F441} Reveal words"}
      </button>
    </div>
  );
}
