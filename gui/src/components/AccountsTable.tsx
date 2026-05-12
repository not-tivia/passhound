import { useState, type ReactNode } from "react";
import type { AccountSummary } from "../types";
import {
  getVaultListColumnOrder,
  getVaultListPrimary,
  setVaultListColumnOrder,
  setVaultListPrimary,
  type ColumnId,
  type PrimaryIdentity,
} from "../preferences";
import TagChip from "./TagChip";

interface AccountsTableProps {
  accounts: AccountSummary[];
  search: string;
  onSearchChange: (s: string) => void;
  selectedId: number | null;
  onSelect: (id: number) => void;
  onLockedError: () => void;
  selectedIds: Set<number>;
  onSelectedIdsChange: (ids: Set<number>) => void;
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
      <div>
        {primaryText ?? <span className="muted">&#x2014;</span>}
        {a.tags && a.tags.length > 0 && (
          <span className="account-row__tags">
            {a.tags.slice(0, 3).map((t) => (
              <TagChip key={t.id} tag={t} />
            ))}
            {a.tags.length > 3 && (
              <span className="tag-chip tag-chip--overflow">+{a.tags.length - 3}</span>
            )}
          </span>
        )}
      </div>
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
  accounts,
  search,
  onSearchChange,
  selectedId,
  onSelect,
  onLockedError: _onLockedError,
  selectedIds,
  onSelectedIdsChange,
}: AccountsTableProps) {
  const [primary, setPrimary] = useState<PrimaryIdentity>(getVaultListPrimary());
  const [columnOrder, setColumnOrder] = useState<ColumnId[]>(getVaultListColumnOrder());
  const [draggedColumn, setDraggedColumn] = useState<ColumnId | null>(null);
  const [dragOverColumn, setDragOverColumn] = useState<ColumnId | null>(null);

  const COLUMNS: Record<ColumnId, ColumnDef> = {
    site: {
      id: "site",
      label: "Site",
      render: (a) => a.site_name,
    },
    user_display: {
      id: "user_display",
      label: "User \xb7 Display",
      render: (a) => renderUserDisplayCell(a, primary),
    },
    last: {
      id: "last",
      label: "Last",
      render: (a) =>
        a.last_changed ? a.last_changed.slice(0, 7) : <span className="muted">&#x2014;</span>,
    },
  };

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

  // Checkbox column helpers
  const visibleIds = accounts.map((a) => a.id);
  const allSelected = visibleIds.length > 0 && visibleIds.every((id) => selectedIds.has(id));
  const someSelected = visibleIds.some((id) => selectedIds.has(id)) && !allSelected;

  const toggleAll = () => {
    if (allSelected) {
      const next = new Set(selectedIds);
      visibleIds.forEach((id) => next.delete(id));
      onSelectedIdsChange(next);
    } else {
      const next = new Set(selectedIds);
      visibleIds.forEach((id) => next.add(id));
      onSelectedIdsChange(next);
    }
  };

  const toggleOne = (id: number) => {
    const next = new Set(selectedIds);
    if (next.has(id)) next.delete(id); else next.add(id);
    onSelectedIdsChange(next);
  };

  return (
    <div className="accounts-table">
      <div className="accounts-table__searchbar">
        <input
          className="accounts-table__search"
          placeholder={`Search ${accounts.length} account${accounts.length === 1 ? "" : "s"}...`}
          value={search}
          onChange={(e) => onSearchChange(e.target.value)}
        />
        <button
          className="accounts-table__primary-toggle"
          title={`Primary: ${primary === "username" ? "username" : "display name"} (click to swap)`}
          onClick={togglePrimary}
        >
          &#x21C5;
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
      <div className="accounts-table__body">
        <table>
          <colgroup>
            <col style={{ width: "28px" }} />
            {columnOrder.map((id) => (
              <col key={id} />
            ))}
          </colgroup>
          <thead>
            <tr>
              {/* Checkbox column — fixed, non-draggable */}
              <th className="accounts-table__check-col">
                <input
                  type="checkbox"
                  ref={(el) => { if (el) el.indeterminate = someSelected; }}
                  checked={allSelected}
                  onChange={toggleAll}
                  aria-label="Select all visible accounts"
                />
              </th>
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
                {/* Checkbox cell — fixed, non-draggable */}
                <td className="accounts-table__check-col" onClick={(e) => e.stopPropagation()}>
                  <input
                    type="checkbox"
                    checked={selectedIds.has(a.id)}
                    onChange={() => toggleOne(a.id)}
                  />
                </td>
                {columnOrder.map((id) => (
                  <td key={id}>{COLUMNS[id].render(a)}</td>
                ))}
              </tr>
            ))}
            {accounts.length === 0 && (
              <tr>
                <td colSpan={columnOrder.length + 1} className="accounts-table__empty">
                  {search ? "No matches" : "No accounts yet — import via CLI or 📥"}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
