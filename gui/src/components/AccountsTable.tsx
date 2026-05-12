import { useEffect, useRef, useState, type ReactNode } from "react";
import { api } from "../api";
import type { AccountSummary, GuiError } from "../types";
import {
  getVaultListColumnOrder,
  getVaultListPrimary,
  setVaultListColumnOrder,
  setVaultListPrimary,
  type ColumnId,
  type PrimaryIdentity,
} from "../preferences";

interface AccountsTableProps {
  selectedId: number | null;
  onSelect: (id: number) => void;
  onLockedError: () => void;
}

interface ColumnDef {
  id: ColumnId;
  label: string;
  render: (a: AccountSummary) => ReactNode;
}

function renderUserDisplayCell(
  a: AccountSummary,
  primary: PrimaryIdentity,
): ReactNode {
  const primaryText = primary === "username" ? a.username : a.display_name;
  const secondaryText = primary === "username" ? a.display_name : a.username;
  return (
    <>
      <div>{primaryText ?? <span className="muted">—</span>}</div>
      {secondaryText && (
        <div className="accounts-table__secondary">{secondaryText}</div>
      )}
    </>
  );
}

/**
 * Move-style reorder: place `from` at `to`'s position; shift others to fill
 * the gap. NOT a swap.
 */
function moveColumn(order: ColumnId[], from: ColumnId, to: ColumnId): ColumnId[] {
  const fromIdx = order.indexOf(from);
  const toIdx = order.indexOf(to);
  if (fromIdx < 0 || toIdx < 0 || fromIdx === toIdx) return order;
  const next = order.slice();
  const [moved] = next.splice(fromIdx, 1);
  next.splice(toIdx, 0, moved);
  return next;
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
  const [primary, setPrimary] = useState<PrimaryIdentity>(getVaultListPrimary());
  const [columnOrder, setColumnOrder] = useState<ColumnId[]>(getVaultListColumnOrder());
  const [draggedColumn, setDraggedColumn] = useState<ColumnId | null>(null);
  const [dragOverColumn, setDragOverColumn] = useState<ColumnId | null>(null);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const COLUMNS: Record<ColumnId, ColumnDef> = {
    site: {
      id: "site",
      label: "Site",
      render: (a) => a.site_name,
    },
    user_display: {
      id: "user_display",
      label: "User · Display",
      render: (a) => renderUserDisplayCell(a, primary),
    },
    last: {
      id: "last",
      label: "Last",
      render: (a) =>
        a.last_changed ? a.last_changed.slice(0, 7) : <span className="muted">—</span>,
    },
  };

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

  const togglePrimary = () => {
    const next: PrimaryIdentity = primary === "username" ? "display_name" : "username";
    setPrimary(next);
    setVaultListPrimary(next);
  };

  const handleColumnDrop = (target: ColumnId) => {
    if (draggedColumn && draggedColumn !== target) {
      const next = moveColumn(columnOrder, draggedColumn, target);
      setColumnOrder(next);
      setVaultListColumnOrder(next);
    }
    setDraggedColumn(null);
    setDragOverColumn(null);
  };

  return (
    <div className="accounts-table">
      <div className="accounts-table__searchbar">
        <input
          className="accounts-table__search"
          placeholder={`Search ${accounts.length} account${accounts.length === 1 ? "" : "s"}...`}
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
        <button
          className="accounts-table__primary-toggle"
          title={`Primary: ${primary === "username" ? "username" : "display name"} (click to swap)`}
          onClick={togglePrimary}
        >
          ⇅
        </button>
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
                {columnOrder.map((id) => {
                  const col = COLUMNS[id];
                  const classes = ["accounts-table__th"];
                  if (draggedColumn === id) classes.push("accounts-table__th--dragging");
                  if (dragOverColumn === id && draggedColumn !== id)
                    classes.push("accounts-table__th--drag-over");
                  return (
                    <th
                      key={id}
                      draggable
                      onDragStart={() => setDraggedColumn(id)}
                      onDragEnter={() => setDragOverColumn(id)}
                      onDragLeave={() => {
                        // Only clear if leaving this specific column
                        if (dragOverColumn === id) setDragOverColumn(null);
                      }}
                      onDragOver={(e) => {
                        e.preventDefault(); // Required to allow drop
                      }}
                      onDrop={() => handleColumnDrop(id)}
                      onDragEnd={() => {
                        setDraggedColumn(null);
                        setDragOverColumn(null);
                      }}
                      className={classes.join(" ")}
                    >
                      {col.label}
                    </th>
                  );
                })}
              </tr>
            </thead>
            <tbody>
              {accounts.map((a) => (
                <tr
                  key={a.id}
                  className={a.id === selectedId ? "accounts-table__row--selected" : ""}
                  onClick={() => onSelect(a.id)}
                >
                  {columnOrder.map((id) => (
                    <td key={id}>{COLUMNS[id].render(a)}</td>
                  ))}
                </tr>
              ))}
              {accounts.length === 0 && (
                <tr>
                  <td colSpan={columnOrder.length} className="accounts-table__empty">
                    {filter ? "No matches" : "No accounts yet — import via CLI or 📥"}
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
