interface IconRailProps {
  onLock: () => void;
}

export default function IconRail({ onLock }: IconRailProps) {
  return (
    <div className="icon-rail">
      <button className="icon-rail__item" title="Lock vault" onClick={onLock}>
        🔐
      </button>
      <button className="icon-rail__item icon-rail__item--active" title="Vault">
        📋
      </button>
      <button
        className="icon-rail__item icon-rail__item--disabled"
        title="Recovery (Phase 4.4)"
        disabled
      >
        🔍
      </button>
      <button
        className="icon-rail__item icon-rail__item--disabled"
        title="Base Words (Phase 4.5)"
        disabled
      >
        ★
      </button>
      <button
        className="icon-rail__item icon-rail__item--disabled"
        title="Settings (Phase 4.6)"
        disabled
      >
        ⚙
      </button>
    </div>
  );
}
