import { useState } from "react";
import { api } from "../api";
import type { EraSummary, EraFormArgs, GuiError } from "../types";

type EraFormModalProps = {
  initial: EraSummary | null;
  onSaved: () => void;
  onClose: () => void;
};

export default function EraFormModal({ initial, onSaved, onClose }: EraFormModalProps) {
  const [name, setName] = useState(initial?.name ?? "");
  const [startDate, setStartDate] = useState(initial?.start_date ?? "");
  const [endDate, setEndDate] = useState(initial?.end_date ?? "");
  const [notes, setNotes] = useState(initial?.notes ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const datesValid = !startDate || !endDate || startDate <= endDate;
  const isValid = name.trim().length > 0 && datesValid;

  const handleSave = async () => {
    if (!isValid || busy) return;
    setBusy(true);
    setError(null);
    try {
      const args: EraFormArgs = {
        name: name.trim(),
        start_date: startDate || null,
        end_date: endDate || null,
        notes: notes.trim() || null,
      };
      if (initial) {
        await api.updateEra(initial.id, args);
      } else {
        await api.addEra(args);
      }
      onSaved();
      onClose();
    } catch (e) {
      const err = e as GuiError;
      setError(err.message ?? err.kind ?? "Failed to save");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal modal--era-form" onClick={(e) => e.stopPropagation()}>
        <h2>{initial ? "Edit era" : "Add era"}</h2>
        <div className="modal__field">
          <label>Name</label>
          <input
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            disabled={busy}
            autoFocus
            required
            onKeyDown={(e) => { if (e.key === "Enter") void handleSave(); }}
          />
        </div>
        <div className="modal__field">
          <label>Start date (optional)</label>
          <input
            type="date"
            value={startDate}
            onChange={(e) => setStartDate(e.target.value)}
            disabled={busy}
          />
        </div>
        <div className="modal__field">
          <label>End date (optional)</label>
          <input
            type="date"
            value={endDate}
            onChange={(e) => setEndDate(e.target.value)}
            disabled={busy}
          />
        </div>
        <div className="modal__field">
          <label>Notes (optional)</label>
          <textarea
            rows={3}
            value={notes}
            onChange={(e) => setNotes(e.target.value)}
            disabled={busy}
          />
        </div>
        {startDate && endDate && !datesValid && (
          <div className="modal__error">End date must be on or after start date.</div>
        )}
        {error && <div className="modal__error">{error}</div>}
        <div className="modal__actions">
          <button onClick={onClose} disabled={busy}>Cancel</button>
          <button onClick={() => void handleSave()} disabled={!isValid || busy}>
            {busy ? "Saving…" : "Save"}
          </button>
        </div>
      </div>
    </div>
  );
}
