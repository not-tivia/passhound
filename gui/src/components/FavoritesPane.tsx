import BaseWordRow from "./BaseWordRow";
import type { BaseWordView } from "../types";

interface FavoritesPaneProps {
  words: BaseWordView[];
  revealAll: boolean;
  onDemote: (id: number) => void;
}

export default function FavoritesPane({ words, revealAll, onDemote }: FavoritesPaneProps) {
  const favorites = words.filter((w) => w.is_favorite);
  return (
    <div className="base-words-pane base-words-pane--favorites">
      <div className="base-words-pane__header">FAVORITES ({favorites.length})</div>
      {favorites.length === 0 ? (
        <div className="base-words-pane__empty">
          No favorites yet. Promote words from the right pane, or click Re-analyze to seed the top 10.
        </div>
      ) : (
        <div className="base-words-pane__list">
          {favorites.map((w) => (
            <BaseWordRow
              key={w.id}
              word={w}
              revealed={revealAll}
              action={{ kind: "demote", onClick: () => onDemote(w.id) }}
            />
          ))}
        </div>
      )}
    </div>
  );
}
