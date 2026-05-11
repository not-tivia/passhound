interface IconRailProps {
  currentView: "list" | "import";
  onLock: () => void;
  onImportClick: () => void;
  onVaultClick: () => void;
}

export default function IconRail({
  currentView,
  onLock,
  onImportClick,
  onVaultClick,
}: IconRailProps) {
  return (
    <div className="icon-rail">
      <button className="icon-rail__item" title="Lock vault" onClick={onLock}>
        🔐
      </button>
      <button
        className={`icon-rail__item ${currentView === "list" ? "icon-rail__item--active" : ""}`}
        title="Vault"
        onClick={onVaultClick}
      >
        📋
      </button>
      <button
        className={`icon-rail__item ${currentView === "import" ? "icon-rail__item--active" : ""}`}
        title="Import"
        onClick={onImportClick}
      >
        📥
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
