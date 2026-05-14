import BaseWordRow from "./BaseWordRow";
import type { BaseWordView } from "../types";

interface AllPaneProps {
  words: BaseWordView[];
  revealAll: boolean;
  search: string;
  onSearchChange: (next: string) => void;
  onPromote: (id: number) => void;
}

export default function AllPane({
  words,
  revealAll,
  search,
  onSearchChange,
  onPromote,
}: AllPaneProps) {
  const q = search.trim().toLowerCase();
  const filtered = q === "" ? words : words.filter((w) => w.word.toLowerCase().includes(q));
  return (
    <div className="base-words-pane base-words-pane--all">
      <div className="base-words-pane__header">ALL ({filtered.length}{q ? ` of ${words.length}` : ""})</div>
      <input
        className="base-words-pane__search"
        type="text"
        placeholder="search words..."
        aria-label="Search base words"
        value={search}
        onChange={(e) => onSearchChange(e.target.value)}
      />
      {words.length === 0 ? (
        <div className="base-words-pane__empty">
          No base words extracted yet. Click {"\u{21BB}"} Re-analyze to extract from your password history.
        </div>
      ) : filtered.length === 0 ? (
        <div className="base-words-pane__empty">No matches.</div>
      ) : (
        <div className="base-words-pane__list">
          {filtered.map((w) => (
            <BaseWordRow
              key={w.id}
              word={w}
              revealed={revealAll}
              action={
                w.is_favorite
                  ? null
                  : { kind: "promote", onClick: () => onPromote(w.id) }
              }
            />
          ))}
        </div>
      )}
    </div>
  );
}
