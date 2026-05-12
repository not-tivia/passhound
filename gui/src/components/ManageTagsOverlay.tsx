import { useEffect, useState } from "react";
import { api } from "../api";
import type { TagWithCount, GuiError } from "../types";

interface ManageTagsOverlayProps {
  onClose: () => void;
  onLockedError: () => void;
  onMutated: () => void;
}

export default function ManageTagsOverlay({ onClose, onLockedError, onMutated }: ManageTagsOverlayProps) {
  const [tags, setTags] = useState<TagWithCount[]>([]);
  const [editing, setEditing] = useState<number | null>(null);
  const [draftName, setDraftName] = useState("");

  const refresh = () => api.listTags().then(setTags, (e) => {
    if ((e as GuiError).kind === "Locked") onLockedError();
  });
  useEffect(() => { refresh(); }, []);

  const saveRename = async (id: number) => {
    try {
      await api.renameTag(id, draftName);
      setEditing(null);
      refresh();
      onMutated();
    } catch (e) {
      if ((e as GuiError).kind === "Locked") onLockedError();
    }
  };

  const confirmDelete = async (t: TagWithCount) => {
    if (!confirm(`Delete tag "${t.name}"? It will be removed from ${t.account_count} account${t.account_count === 1 ? "" : "s"}.`)) return;
    try {
      await api.deleteTag(t.id);
      refresh();
      onMutated();
    } catch (e) {
      if ((e as GuiError).kind === "Locked") onLockedError();
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal modal--manage-tags" onClick={(e) => e.stopPropagation()}>
        <h2>Manage tags</h2>
        {tags.length === 0 && <p className="modal__empty">No tags yet.</p>}
        <ul className="manage-tags__list">
          {tags.map((t) => (
            <li key={t.id} className="manage-tags__row">
              {editing === t.id ? (
                <>
                  <input
                    autoFocus
                    value={draftName}
                    onChange={(e) => setDraftName(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") saveRename(t.id);
                      else if (e.key === "Escape") setEditing(null);
                    }}
                  />
                  <button onClick={() => saveRename(t.id)}>Save</button>
                  <button onClick={() => setEditing(null)}>Cancel</button>
                </>
              ) : (
                <>
                  <span className="manage-tags__name">{t.name}</span>
                  <span className="manage-tags__count">({t.account_count} account{t.account_count === 1 ? "" : "s"})</span>
                  <button onClick={() => { setEditing(t.id); setDraftName(t.name); }}>edit</button>
                  <button onClick={() => confirmDelete(t)} className="manage-tags__delete">delete</button>
                </>
              )}
            </li>
          ))}
        </ul>
        <div className="modal__actions">
          <button onClick={onClose}>Close</button>
        </div>
      </div>
    </div>
  );
}
