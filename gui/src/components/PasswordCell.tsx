import { useState } from "react";
import { api } from "../api";
import { useToast } from "./Toast";
import { useSettings } from "../context/SettingsContext";
import type { GuiError } from "../types";

interface PasswordCellProps {
  historyId: number;
  onLockedError: () => void;
  onDelete: () => void;
  onPromote?: () => void;
  onEditCurrent?: (newPlaintext: string) => Promise<void>;
}

export default function PasswordCell({ historyId, onLockedError, onDelete, onPromote, onEditCurrent }: PasswordCellProps) {
  const [revealed, setRevealed] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState(false);
  const [editValue, setEditValue] = useState("");
  const [editBusy, setEditBusy] = useState(false);
  const toast = useToast();
  const { settings } = useSettings();

  const fetchPlaintext = async (): Promise<string | null> => {
    setBusy(true);
    try {
      return await api.revealPassword(historyId);
    } catch (e) {
      const err = e as GuiError;
      if (err.kind === "Locked") onLockedError();
      else toast.show(`Error: ${err.message ?? err.kind}`);
      return null;
    } finally {
      setBusy(false);
    }
  };

  const handleReveal = async () => {
    if (revealed !== null) {
      setRevealed(null);
      return;
    }
    const pt = await fetchPlaintext();
    if (pt !== null) setRevealed(pt);
  };

  const handleCopy = async () => {
    let pt = revealed;
    if (pt === null) pt = await fetchPlaintext();
    if (pt === null) return;
    try {
      await api.copyToClipboardWithAutoClear(pt, settings.clipboard_clear_seconds);
      toast.show("Copied");
    } catch (e) {
      const err = e as GuiError;
      toast.show(`Copy failed: ${err.message ?? err.kind}`);
    }
  };

  const handleDelete = async () => {
    if (!confirm("Delete this password entry? This cannot be undone.")) return;
    setBusy(true);
    try {
      await api.deletePassword(historyId);
      onDelete();
    } catch (e) {
      const err = e as GuiError;
      if (err.kind === "Locked") onLockedError();
      else toast.show(`Delete failed: ${err.message ?? err.kind}`);
    } finally {
      setBusy(false);
    }
  };

  const handleEditOpen = async () => {
    let pt = revealed;
    if (pt === null) {
      pt = await fetchPlaintext();
      if (pt === null) return;
      setRevealed(pt);
    }
    setEditValue(pt);
    setEditing(true);
  };

  const handleSave = async () => {
    if (!editValue.trim()) return;
    setEditBusy(true);
    try {
      await onEditCurrent!(editValue);
      setEditing(false);
      setEditValue("");
    } catch {
      // The parent's onEditCurrent callback handles toast / error display.
      // If it threw, we stay in editing mode so the user can retry.
    } finally {
      setEditBusy(false);
    }
  };

  if (editing) {
    return (
      <div className="pwd-cell">
        <input
          type="text"
          className="pwd-cell__edit-input"
          value={editValue}
          onChange={(e) => setEditValue(e.target.value)}
          disabled={editBusy}
          autoFocus
        />
        <button className="pwd-cell__btn" onClick={handleSave} disabled={editBusy || !editValue.trim()}>
          Save
        </button>
        <button
          className="pwd-cell__btn"
          onClick={() => { setEditing(false); setEditValue(""); }}
          disabled={editBusy}
        >
          Cancel
        </button>
      </div>
    );
  }

  return (
    <div className="pwd-cell">
      <span className="pwd-cell__value">
        {revealed ?? "••••••••••••"}
      </span>
      <button className="pwd-cell__btn" onClick={handleReveal} disabled={busy}>
        {revealed !== null ? "hide" : "reveal"}
      </button>
      <button className="pwd-cell__btn" onClick={handleCopy} disabled={busy}>
        copy
      </button>
      {onEditCurrent && (
        <button
          className="pwd-cell__btn pwd-cell__btn--edit"
          onClick={handleEditOpen}
          disabled={busy}
          aria-label="Edit current password"
          title="Edit current password"
        >
          &#x270F; Edit
        </button>
      )}
      {onPromote && (
        <button
          className="pwd-cell__btn pwd-cell__btn--promote"
          onClick={onPromote}
          disabled={busy}
          aria-label="Set as current password"
          title="Set as current password"
        >
          {"↑"}
        </button>
      )}
      <button
        className="pwd-cell__btn pwd-cell__btn--delete"
        onClick={handleDelete}
        disabled={busy}
        aria-label="Delete password"
        title="Delete this password"
      >
        &times;
      </button>
    </div>
  );
}
