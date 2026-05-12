import { useEffect, useState } from "react";
import IconRail from "../components/IconRail";
import AccountsTable from "../components/AccountsTable";
import TagsSidebar from "../components/TagsSidebar";
import PerSite from "./PerSite";
import Import from "./Import";
import { ToastProvider } from "../components/Toast";

interface VaultProps {
  onLock: () => void;
}

type View = "list" | "import";

export default function Vault({ onLock }: VaultProps) {
  const [view, setView] = useState<View>("list");
  const [selectedId, setSelectedId] = useState<number | null>(null);
  // Bump this to force AccountsTable to re-fetch after a successful import.
  const [refreshKey, setRefreshKey] = useState(0);

  const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set());
  const [filterTagIds, setFilterTagIds] = useState<number[]>([]);
  const [_manageOpen, setManageOpen] = useState(false);

  // Clear row-level selection whenever the tag filter changes (selected rows may
  // no longer be visible after the filter is applied).
  useEffect(() => { setSelectedIds(new Set()); }, [filterTagIds]);

  return (
    <ToastProvider>
      <div className="vault-shell">
        <IconRail
          currentView={view}
          onLock={onLock}
          onImportClick={() => setView("import")}
          onVaultClick={() => setView("list")}
        />
        {view === "list" && (
          <div className="vault-grid">
            <TagsSidebar
              filterTagIds={filterTagIds}
              onFilterChange={setFilterTagIds}
              onManageClick={() => setManageOpen(true)}
              onLockedError={onLock}
              refreshKey={refreshKey}
            />
            <div className="vault-accounts">
              <AccountsTable
                filterTagIds={filterTagIds}
                selectedIds={selectedIds}
                onSelectedIdsChange={setSelectedIds}
                selectedId={selectedId}
                onSelect={setSelectedId}
                refreshKey={refreshKey}
                onLockedError={onLock}
              />
              {/* BulkActionBar slot — added in Task 8 */}
            </div>
            <div className="vault-detail">
              {selectedId === null ? (
                <div className="vault-empty">Select an account from the list.</div>
              ) : (
                <PerSite
                  accountId={selectedId}
                  onLockedError={onLock}
                  onAccountDeleted={() => {
                    setSelectedId(null);
                    setRefreshKey((k) => k + 1);
                  }}
                />
              )}
            </div>
            {/* ManageTagsOverlay slot — added in Task 9 */}
          </div>
        )}
        {view === "import" && (
          <Import
            onDone={() => {
              setView("list");
              setRefreshKey((k) => k + 1);
            }}
            onLockedError={onLock}
          />
        )}
      </div>
    </ToastProvider>
  );
}
