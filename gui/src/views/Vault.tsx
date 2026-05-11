import { useState } from "react";
import IconRail from "../components/IconRail";
import AccountsTable from "../components/AccountsTable";

interface VaultProps {
  onLock: () => void;
}

export default function Vault({ onLock }: VaultProps) {
  const [selectedId, setSelectedId] = useState<number | null>(null);

  return (
    <div className="vault-shell">
      <IconRail onLock={onLock} />
      <div className="vault-split">
        <div className="vault-split__left">
          <AccountsTable
            selectedId={selectedId}
            onSelect={setSelectedId}
            onLockedError={onLock}
          />
        </div>
        <div className="vault-split__right">
          {selectedId === null ? (
            <div className="vault-empty">Select an account from the list.</div>
          ) : (
            <div className="vault-empty">Per-site detail lands in Task 6 (id: {selectedId})</div>
          )}
        </div>
      </div>
    </div>
  );
}
