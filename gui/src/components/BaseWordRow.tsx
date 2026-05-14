import type { BaseWordView } from "../types";

const MASK = "\u{2022}".repeat(8);

interface BaseWordRowProps {
  word: BaseWordView;
  revealed: boolean;
  action: { kind: "demote" | "promote"; onClick: () => void } | null;
}

export default function BaseWordRow({ word, revealed, action }: BaseWordRowProps) {
  return (
    <div className="base-word-row">
      <span className="base-word-row__star">{word.is_favorite ? "\u{2605}" : "\u{2606}"}</span>
      <span className="base-word-row__word">{revealed ? word.word : MASK}</span>
      <span className="base-word-row__count">{word.usage_count}</span>
      <span className="base-word-row__badge">
        {word.manual_override ? "manual" : ""}
      </span>
      <span className="base-word-row__action">
        {action?.kind === "demote" && (
          <button
            className="base-word-row__btn base-word-row__btn--demote"
            onClick={action.onClick}
            aria-label="Demote from favorites"
            title="Demote from favorites"
          >
            {"\u{2715}"}
          </button>
        )}
        {action?.kind === "promote" && (
          <button
            className="base-word-row__btn base-word-row__btn--promote"
            onClick={action.onClick}
            aria-label="Promote to favorites"
            title="Promote to favorites"
          >
            +
          </button>
        )}
      </span>
    </div>
  );
}
