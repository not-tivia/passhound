import { useState } from "react";
import TagPicker from "./TagPicker";
import DeleteAccountsConfirm from "./DeleteAccountsConfirm";
import { api } from "../api";
import type { AccountSummary, GuiError, TagSummary } from "../types";

interface BulkActionBarProps {
  selectedIds: Set<number>;
  accounts: AccountSummary[];
  onSelectedIdsChange: (ids: Set<number>) => void;
  onAfterMutation: () => void;
  onLockedError: () => void;
}

export default function BulkActionBar({
  selectedIds, accounts, onSelectedIdsChange, onAfterMutation, onLockedError,
}: BulkActionBarProps) {
  const [picker, setPicker] = useState<null | "add" | "remove">(null);
  const [showDelete, setShowDelete] = useState(false);

  if (selectedIds.size === 0) return null;
  const ids = Array.from(selectedIds);

  // Tags present on at least one selected account, for the "Remove tag" picker.
  const selectedAccountsTags = new Map<number, TagSummary>();
  for (const a of accounts) {
    if (!selectedIds.has(a.id)) continue;
    for (const t of a.tags ?? []) {
      selectedAccountsTags.set(t.id, t);
    }
  }
  const removeSuggestions = Array.from(selectedAccountsTags.values()).sort((a, b) => a.name.localeCompare(b.name));

  const handleAdd = async (chosen: TagSummary | { name: string; isNew: true }) => {
    setPicker(null);
    try {
      let tagId: number;
      if ("isNew" in chosen) {
        const created = await api.createTag(chosen.name);
        tagId = created.id;
      } else {
        tagId = chosen.id;
      }
      await api.bulkAssignTag(ids, tagId);
      onAfterMutation();
    } catch (e) {
      if ((e as GuiError).kind === "Locked") onLockedError();
    }
  };

  const handleRemove = async (chosen: TagSummary | { name: string; isNew: true }) => {
    setPicker(null);
    if ("isNew" in chosen) return; // Cannot remove a tag that doesn't exist.
    try {
      await api.bulkUnassignTag(ids, chosen.id);
      onAfterMutation();
    } catch (e) {
      if ((e as GuiError).kind === "Locked") onLockedError();
    }
  };

  const handleDelete = async () => {
    setShowDelete(false);
    try {
      await api.bulkDeleteAccounts(ids);
      onSelectedIdsChange(new Set());
      onAfterMutation();
    } catch (e) {
      if ((e as GuiError).kind === "Locked") onLockedError();
    }
  };

  return (
    <>
      <div className="bulk-bar">
        <span className="bulk-bar__count">{selectedIds.size} selected</span>
        <button onClick={() => setPicker("add")}>+ Add tag</button>
        <button onClick={() => setPicker("remove")} disabled={removeSuggestions.length === 0}>&#x2212; Remove tag</button>
        <button className="bulk-bar__danger" onClick={() => setShowDelete(true)}>&#x1F5D1; Delete</button>
        <button className="bulk-bar__clear" onClick={() => onSelectedIdsChange(new Set())}>&#x00D7; Clear</button>

        {picker === "add" && (
          <div className="bulk-bar__popover">
            <TagPicker
              onSelect={handleAdd}
              onCancel={() => setPicker(null)}
              onLockedError={onLockedError}
              placeholder="add tag&#x2026;"
            />
          </div>
        )}
        {picker === "remove" && (
          <div className="bulk-bar__popover">
            <TagPicker
              suggestions={removeSuggestions}
              onSelect={handleRemove}
              onCancel={() => setPicker(null)}
              onLockedError={onLockedError}
              placeholder="remove tag&#x2026;"
            />
          </div>
        )}
      </div>
      {showDelete && (
        <DeleteAccountsConfirm
          count={selectedIds.size}
          onConfirm={handleDelete}
          onCancel={() => setShowDelete(false)}
        />
      )}
    </>
  );
}
