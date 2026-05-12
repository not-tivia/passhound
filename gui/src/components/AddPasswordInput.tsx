import { useState } from "react";
import { api } from "../api";
import type { GuiError } from "../types";

interface AddPasswordInputProps {
  accountId: number;
  onSave: () => void;
  onCancel: () => void;
  onLockedError: () => void;
}

export default function AddPasswordInput({ accountId, onSave, onCancel, onLockedError }: AddPasswordInputProps) {
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);

  const handleSave = async () => {
    if (!value || busy) return;
    setBusy(true);
    try {
      await api.addPassword(accountId, value);
      onSave();
    } catch (e) {
      const err = e as GuiError;
      if (err.kind === "Locked") onLockedError();
      else alert(`Failed to add password: ${err.message ?? err.kind}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="add-password-input">
      <input
        autoFocus
        type="text"
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") { e.preventDefault(); handleSave(); }
          else if (e.key === "Escape") { e.preventDefault(); onCancel(); }
        }}
        placeholder="password"
      />
      <button onClick={handleSave} disabled={!value || busy}>Save</button>
      <button onClick={onCancel} disabled={busy}>Cancel</button>
    </div>
  );
}
