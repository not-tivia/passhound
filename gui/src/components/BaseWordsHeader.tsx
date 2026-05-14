interface BaseWordsHeaderProps {
  total: number;
  revealAll: boolean;
  analyzing: boolean;
  onToggleReveal: () => void;
  onReanalyze: () => void;
}

export default function BaseWordsHeader({
  total,
  revealAll,
  analyzing,
  onToggleReveal,
  onReanalyze,
}: BaseWordsHeaderProps) {
  return (
    <div className="base-words-header">
      <span className="base-words-header__title">Base Words</span>
      <button
        className="base-words-header__reanalyze"
        onClick={onReanalyze}
        disabled={analyzing}
      >
        {analyzing ? "Analyzing…" : "\u{21BB} Re-analyze"}
      </button>
      <span className="base-words-header__count">{total} total</span>
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
