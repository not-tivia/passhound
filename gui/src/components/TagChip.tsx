import type { TagSummary } from "../types";

interface TagChipProps {
  tag: TagSummary;
  onRemove?: () => void;
  onClick?: () => void;
  active?: boolean;
}

export default function TagChip({ tag, onRemove, onClick, active }: TagChipProps) {
  return (
    <span
      className={`tag-chip${active ? " tag-chip--active" : ""}${onClick ? " tag-chip--clickable" : ""}`}
      onClick={onClick}
      role={onClick ? "button" : undefined}
    >
      <span className="tag-chip__name">{tag.name}</span>
      {onRemove && (
        <button
          className="tag-chip__remove"
          onClick={(e) => { e.stopPropagation(); onRemove(); }}
          aria-label={`Remove tag ${tag.name}`}
          title="Remove tag"
        >
          &times;
        </button>
      )}
    </span>
  );
}
