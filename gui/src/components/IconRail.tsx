interface IconRailProps {
  currentView: "list" | "import" | "recovery" | "base-words" | "settings";
  onLock: () => void;
  onImportClick: () => void;
  onVaultClick: () => void;
  onRecoveryClick: () => void;
  onBaseWordsClick: () => void;
  onSettingsClick: () => void;
}

export default function IconRail({
  currentView,
  onLock,
  onImportClick,
  onVaultClick,
  onRecoveryClick,
  onBaseWordsClick,
  onSettingsClick,
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
        className={`icon-rail__item ${currentView === "settings" ? "icon-rail__item--active" : ""}`}
        title="Settings"
        onClick={onSettingsClick}
      >
        {"\u{2699}"}
      </button>
    </div>
  );
}
