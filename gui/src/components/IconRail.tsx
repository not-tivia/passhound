interface IconRailProps {
  currentView: "list" | "import" | "recovery";
  onLock: () => void;
  onImportClick: () => void;
  onVaultClick: () => void;
  onRecoveryClick: () => void;
}

export default function IconRail({
  currentView,
  onLock,
  onImportClick,
  onVaultClick,
  onRecoveryClick,
}: IconRailProps) {
  return (
    <div className="icon-rail">
      <button className="icon-rail__item" title="Lock vault" onClick={onLock}>
        {"\u{1F510}"}
      </button>
      <button
        className={`icon-rail__item ${currentView === "list" ? "icon-rail__item--active" : ""}`}
        title="Vault"
        onClick={onVaultClick}
      >
        {"\u{1F4CB}"}
      </button>
      <button
        className={`icon-rail__item ${currentView === "import" ? "icon-rail__item--active" : ""}`}
        title="Import"
        onClick={onImportClick}
      >
        {"\u{1F4E5}"}
      </button>
      <button
        className={`icon-rail__item ${currentView === "recovery" ? "icon-rail__item--active" : ""}`}
        title="Recovery"
        onClick={onRecoveryClick}
      >
        {"\u{1F50D}"}
      </button>
      <button
        className="icon-rail__item icon-rail__item--disabled"
        title="Base Words (Phase 4.9)"
        disabled
      >
        {"★"}
      </button>
      <button
        className="icon-rail__item icon-rail__item--disabled"
        title="Settings (Phase 4.10)"
        disabled
      >
        {"⚙"}
      </button>
    </div>
  );
}
