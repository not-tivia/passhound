import { useState } from "react";
import IconRail from "../components/IconRail";
import AccountsTable from "../components/AccountsTable";
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
          <div className="vault-split">
            <div className="vault-split__left">
              <AccountsTable
                key={refreshKey}
                selectedId={selectedId}
                onSelect={setSelectedId}
                onLockedError={onLock}
              />
            </div>
            <div className="vault-split__right">
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
