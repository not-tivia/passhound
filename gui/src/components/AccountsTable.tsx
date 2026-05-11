import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { AccountSummary, GuiError } from "../types";

interface AccountsTableProps {
  selectedId: number | null;
  onSelect: (id: number) => void;
  onLockedError: () => void;
}

export default function AccountsTable({
  selectedId,
  onSelect,
  onLockedError,
}: AccountsTableProps) {
  const [accounts, setAccounts] = useState<AccountSummary[]>([]);
  const [filter, setFilter] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const load = async (q: string) => {
    setLoading(true);
    setError(null);
    try {
      const list = await api.listAccounts(q || undefined);
      setAccounts(list);
    } catch (e) {
      const err = e as GuiError;
      if (err.kind === "Locked") {
        onLockedError();
        return;
      }
      setError(err.message ?? err.kind);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load("");
  }, []);

  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => load(filter), 250);
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, [filter]);

  return (
    <div className="accounts-table">
      <div className="accounts-table__searchbar">
        <input
          className="accounts-table__search"
          placeholder={`Search ${accounts.length} account${accounts.length === 1 ? "" : "s"}...`}
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
      </div>
      <div className="accounts-table__filters">
        <button className="accounts-table__pill accounts-table__pill--active">All</button>
        <button className="accounts-table__pill" disabled title="Coming later">
          Categories
        </button>
        <button className="accounts-table__pill" disabled title="Coming later">
          Eras
        </button>
      </div>
      {loading && <div className="accounts-table__status">Loading...</div>}
      {error && <div className="accounts-table__status accounts-table__status--error">{error}</div>}
      {!loading && !error && (
        <div className="accounts-table__body">
          <table>
            <thead>
              <tr>
                <th>Site</th>
                <th>User</th>
                <th>Last</th>
              </tr>
            </thead>
            <tbody>
              {accounts.map((a) => (
                <tr
                  key={a.id}
                  className={a.id === selectedId ? "accounts-table__row--selected" : ""}
                  onClick={() => onSelect(a.id)}
                >
                  <td>{a.site_name}</td>
                  <td>{a.username ?? <span className="muted">—</span>}</td>
                  <td>{a.last_changed ? a.last_changed.slice(0, 7) : <span className="muted">—</span>}</td>
                </tr>
              ))}
              {accounts.length === 0 && (
                <tr>
                  <td colSpan={3} className="accounts-table__empty">
                    {filter ? "No matches" : "No accounts yet — import via CLI"}
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
