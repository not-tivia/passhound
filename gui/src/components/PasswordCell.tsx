import { useState } from "react";
import { api } from "../api";
import { useToast } from "./Toast";
import type { GuiError } from "../types";

interface PasswordCellProps {
  historyId: number;
  onLockedError: () => void;
}

export default function PasswordCell({ historyId, onLockedError }: PasswordCellProps) {
  const [revealed, setRevealed] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const toast = useToast();

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
      await api.copyToClipboard(pt);
      toast.show("Copied");
    } catch (e) {
      const err = e as GuiError;
      toast.show(`Copy failed: ${err.message ?? err.kind}`);
    }
  };

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
    </div>
  );
}
