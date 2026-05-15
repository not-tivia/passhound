import { useEffect, useState } from "react";
import IconRail from "../components/IconRail";
import AccountsTable from "../components/AccountsTable";
import TagsSidebar from "../components/TagsSidebar";
import BulkActionBar from "../components/BulkActionBar";
import ManageTagsOverlay from "../components/ManageTagsOverlay";
import AccountFormModal from "../components/AccountFormModal";
import PerSite from "./PerSite";
import Import from "./Import";
import Recovery from "./Recovery";
import BaseWords from "./BaseWords";
import Settings from "./Settings";
import { ToastProvider } from "../components/Toast";
import { api } from "../api";
import type { AccountSummary, GuiError } from "../types";

interface VaultProps {
  onLock: () => void;
}

type View = "list" | "import" | "recovery" | "base-words" | "settings";

export default function Vault({ onLock }: VaultProps) {
  const [view, setView] = useState<View>("list");
  const [selectedId, setSelectedId] = useState<number | null>(null);
  // Bump this to force a re-fetch after a successful import or mutation.
  const [refreshKey, setRefreshKey] = useState(0);
  const [recoveryInitial, setRecoveryInitial] = useState<{ site?: string; account?: string } | undefined>(undefined);

  const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set());
  const [filterTagIds, setFilterTagIds] = useState<number[]>([]);
  const [manageOpen, setManageOpen] = useState(false);
  const [addingAccount, setAddingAccount] = useState(false);

  // Lifted from AccountsTable so BulkActionBar can read the same accounts array.
  const [accounts, setAccounts] = useState<AccountSummary[]>([]);
  const [search, setSearch] = useState("");

  // Clear row-level selection whenever the tag filter changes (selected rows may
  // no longer be visible after the filter is applied).
  useEffect(() => { setSelectedIds(new Set()); }, [filterTagIds]);

  // Immediate (non-debounced) reload on filter or refreshKey change.
  useEffect(() => {
    api.listAccounts(search || undefined, filterTagIds.length > 0 ? filterTagIds : undefined).then(
      setAccounts,
      (e) => { if ((e as GuiError).kind === "Locked") onLock(); }
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refreshKey, filterTagIds, onLock]);

  // Debounced reload on search-text change.
  useEffect(() => {
    const t = setTimeout(() => {
      api.listAccounts(search || undefined, filterTagIds.length > 0 ? filterTagIds : undefined).then(
        setAccounts,
        (e) => { if ((e as GuiError).kind === "Locked") onLock(); }
      );
    }, 300);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [search]);

  return (
    <ToastProvider>
      <div className="vault-shell">
        <IconRail
          currentView={view}
          onLock={onLock}
          onImportClick={() => setView("import")}
          onVaultClick={() => setView("list")}
          onRecoveryClick={() => {
            setRecoveryInitial(undefined);
            setView("recovery");
          }}
          onBaseWordsClick={() => setView("base-words")}
          onSettingsClick={() => setView("settings")}
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
              <button className="vault__add-account-btn" onClick={() => setAddingAccount(true)}>
                + Add account
              </button>
              <AccountsTable
                accounts={accounts}
                search={search}
                onSearchChange={setSearch}
                selectedIds={selectedIds}
                onSelectedIdsChange={setSelectedIds}
                selectedId={selectedId}
                onSelect={setSelectedId}
                onLockedError={onLock}
              />
              <BulkActionBar
                selectedIds={selectedIds}
                accounts={accounts}
                onSelectedIdsChange={setSelectedIds}
                onAfterMutation={() => setRefreshKey((k) => k + 1)}
                onLockedError={onLock}
              />
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
                  onRecoverAccount={(siteName, accountLabel) => {
                    setRecoveryInitial({ site: siteName, account: accountLabel ?? undefined });
                    setView("recovery");
                  }}
                />
              )}
            </div>
            {addingAccount && (
              <AccountFormModal
                mode="add"
                onClose={() => setAddingAccount(false)}
                onSaved={(id) => {
                  setAddingAccount(false);
                  setRefreshKey((k) => k + 1);
                  setSelectedId(id);
                }}
                onLockedError={onLock}
              />
            )}
            {manageOpen && (
              <ManageTagsOverlay
                onClose={() => setManageOpen(false)}
                onLockedError={onLock}
                onMutated={() => {
                  setRefreshKey((k) => k + 1);
                  setFilterTagIds([]); // Drop any filter that may now point at a renamed/deleted tag.
                }}
              />
            )}
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
        {view === "recovery" && (
          <Recovery
            initial={recoveryInitial}
            onLockedError={onLock}
            onNavigateToImport={() => setView("import")}
          />
        )}
        {view === "base-words" && (
          <BaseWords onLockedError={onLock} />
        )}
        {view === "settings" && (
          <Settings onLockedError={onLock} />
        )}
      </div>
    </ToastProvider>
  );
}
