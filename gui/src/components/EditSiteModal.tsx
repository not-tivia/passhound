import { useState } from "react";
import { api } from "../api";
import { useToast } from "./Toast";
import type { AccountDetail, GuiError } from "../types";

interface EditSiteModalProps {
  detail: AccountDetail;
  onClose: () => void;
  onSaved: () => void;
  onLockedError: () => void;
}

export default function EditSiteModal({ detail, onClose, onSaved, onLockedError }: EditSiteModalProps) {
  const toast = useToast();
  const [name, setName] = useState(detail.site_name);
  const [url, setUrl] = useState(detail.site_url ?? "");
  const [category, setCategory] = useState(detail.site_category ?? "");
  const [abbreviations, setAbbreviations] = useState(detail.site_abbreviations.join(", "));
  const [notes, setNotes] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSave = async () => {
    if (!name.trim()) {
      setError("Name is required.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const abbrList = abbreviations
        .split(",")
        .map((s) => s.trim())
        .filter((s) => s.length > 0);
      await api.updateSite(detail.site_id, {
        name: name.trim(),
        url: url.trim() || null,
        category: category.trim() || null,
        abbreviations: abbrList,
        notes: notes.trim() || null,
      });
      toast.show("Site updated");
      onSaved();
      onClose();
    } catch (e) {
      const err = e as GuiError;
      if (err.kind === "Locked") onLockedError();
      else setError(err.message ?? err.kind);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal modal--edit-site" onClick={(e) => e.stopPropagation()}>
        <h2>Edit site</h2>
        <div className="modal__field">
          <label>Name</label>
          <input value={name} onChange={(e) => setName(e.target.value)} disabled={busy} autoFocus />
        </div>
        <div className="modal__field">
          <label>URL</label>
          <input value={url} onChange={(e) => setUrl(e.target.value)} disabled={busy} />
        </div>
        <div className="modal__field">
          <label>Category</label>
          <input value={category} onChange={(e) => setCategory(e.target.value)} disabled={busy} />
        </div>
        <div className="modal__field">
          <label>Abbreviations (comma-separated)</label>
          <input value={abbreviations} onChange={(e) => setAbbreviations(e.target.value)} disabled={busy} />
        </div>
        <div className="modal__field">
          <label>Notes</label>
          <textarea value={notes} onChange={(e) => setNotes(e.target.value)} disabled={busy} rows={3} />
        </div>
        {error && <div className="modal__error">{error}</div>}
        <div className="modal__actions">
          <button onClick={onClose} disabled={busy}>Cancel</button>
          <button onClick={handleSave} disabled={busy}>{busy ? "Saving…" : "Save"}</button>
        </div>
      </div>
    </div>
  );
}
