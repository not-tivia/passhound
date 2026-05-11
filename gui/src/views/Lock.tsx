import { useEffect, useState } from "react";
import { api } from "../api";
import type { GuiError } from "../types";

interface LockProps {
  onUnlock: () => void;
}

export default function Lock({ onUnlock }: LockProps) {
  const [mode, setMode] = useState<"loading" | "unlock" | "create">("loading");
  const [pw, setPw] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    api
      .vaultExists()
      .then((exists) => setMode(exists ? "unlock" : "create"))
      .catch(() => setMode("create"));
  }, []);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    if (!pw) {
      setError("Password required");
      return;
    }
    if (mode === "create" && pw !== confirm) {
      setError("Passwords don't match");
      return;
    }
    setSubmitting(true);
    try {
      if (mode === "create") {
        await api.vaultCreate(pw);
      } else {
        await api.vaultUnlock(pw);
      }
      // Clear inputs immediately on success — no plaintext password lingering
      // in component state across the lifecycle of the next view.
      setPw("");
      setConfirm("");
      onUnlock();
    } catch (e) {
      const err = e as GuiError;
      if (err.kind === "WrongPassword") setError("Wrong password");
      else if (err.kind === "AlreadyExists") setError("Vault already exists at the default path");
      else setError(err.message ?? `error: ${err.kind}`);
    } finally {
      setSubmitting(false);
    }
  };

  if (mode === "loading") {
    return (
      <div className="lock-shell">
        <div className="lock-card">
          <div className="lock-icon">🔐</div>
          <div className="lock-title">PassHound</div>
          <div className="lock-subtitle">Loading...</div>
        </div>
      </div>
    );
  }

  return (
    <div className="lock-shell">
      <form className="lock-card" onSubmit={handleSubmit}>
        <div className="lock-icon">🔐</div>
        <div className="lock-title">PassHound</div>
        <div className="lock-subtitle">
          {mode === "create" ? "Create your vault" : "Unlock vault"}
        </div>

        <input
          type="password"
          className="lock-input"
          placeholder="Master password"
          value={pw}
          onChange={(e) => setPw(e.target.value)}
          autoFocus
          disabled={submitting}
        />

        {mode === "create" && (
          <input
            type="password"
            className="lock-input"
            placeholder="Confirm password"
            value={confirm}
            onChange={(e) => setConfirm(e.target.value)}
            disabled={submitting}
          />
        )}

        {mode === "create" && (
          <div className="lock-warning">
            ⚠ This password cannot be recovered. Pick something memorable.
          </div>
        )}

        {error && <div className="lock-error">{error}</div>}

        <button type="submit" className="lock-button" disabled={submitting}>
          {submitting ? "..." : mode === "create" ? "Create vault" : "Unlock"}
        </button>

        <div className="lock-path">
          Vault: ~/.local/share/passhound/vault.db
        </div>
      </form>
    </div>
  );
}
