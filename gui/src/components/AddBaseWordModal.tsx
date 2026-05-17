import { useState } from "react";
import { api } from "../api";
import { useToast } from "./Toast";
import type { GuiError } from "../types";

interface AddBaseWordModalProps {
  onClose: () => void;
  onAdded: () => void;
  onLockedError: () => void;
}

export default function AddBaseWordModal({ onClose, onAdded, onLockedError }: AddBaseWordModalProps) {
  const toast = useToast();
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleAdd = async () => {
    const trimmed = text.trim();
    if (!trimmed) {
      setError("Enter a word.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const view = await api.addBaseWord(trimmed);
      toast.show(`Added '${view.word}' to favorites`);
      onAdded();
      onClose();
    } catch (e) {
      const err = e as GuiError;
      if (err.kind === "Locked") onLockedError();
      else if (err.kind === "AlreadyExists") setError("That word is already in your list.");
      else setError(err.message ?? err.kind);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal modal--add-base-word" onClick={(e) => e.stopPropagation()}>
        <h2>Add base word</h2>
        <div className="modal__field">
          <label>Word</label>
          <input
            value={text}
            onChange={(e) => setText(e.target.value)}
            disabled={busy}
            autoFocus
            onKeyDown={(e) => { if (e.key === "Enter") void handleAdd(); }}
          />
        </div>
        {error && <div className="modal__error">{error}</div>}
        <div className="modal__actions">
          <button onClick={onClose} disabled={busy}>Cancel</button>
          <button onClick={() => void handleAdd()} disabled={busy}>{busy ? "Adding…" : "Add"}</button>
        </div>
      </div>
    </div>
  );
}
