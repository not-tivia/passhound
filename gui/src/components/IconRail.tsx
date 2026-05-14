interface IconRailProps {
  currentView: "list" | "import" | "recovery" | "base-words";
  onLock: () => void;
  onImportClick: () => void;
  onVaultClick: () => void;
  onRecoveryClick: () => void;
  onBaseWordsClick: () => void;
}

export default function IconRail({
  currentView,
  onLock,
  onImportClick,
  onVaultClick,
  onRecoveryClick,
  onBaseWordsClick,
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
        className={`icon-rail__item ${currentView === "base-words" ? "icon-rail__item--active" : ""}`}
        title="Base Words"
        onClick={onBaseWordsClick}
      >
        {"\u{2605}"}
      </button>
      <button
        className="icon-rail__item icon-rail__item--disabled"
        title="Settings (Phase 4.10)"
        disabled
      >
        {"\u{2699}"}
      </button>
    </div>
  );
}
