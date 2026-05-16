import { useState } from "react";
import { api } from "../api";
import PasswordGeneratorPopover from "./PasswordGeneratorPopover";
import type { GuiError } from "../types";

interface ChangeMasterPasswordModalProps {
  onClose: () => void;
  onChanged: () => void;
  onLockedError: () => void;
}

export default function ChangeMasterPasswordModal({
  onClose,
  onChanged,
  onLockedError,
}: ChangeMasterPasswordModalProps) {
  const [currentPw, setCurrentPw] = useState("");
  const [newPw, setNewPw] = useState("");
  const [confirmPw, setConfirmPw] = useState("");
  const [busy, setBusy] = useState(false);
  const [genOpen, setGenOpen] = useState(false);
  const [errorCurrent, setErrorCurrent] = useState<string | null>(null);
  const [errorNew, setErrorNew] = useState<string | null>(null);
  const [errorConfirm, setErrorConfirm] = useState<string | null>(null);
  const [errorGeneral, setErrorGeneral] = useState<string | null>(null);

  const handleChange = async () => {
    if (busy) return;
    setErrorCurrent(null);
    setErrorNew(null);
    setErrorConfirm(null);
    setErrorGeneral(null);

    // Client-side validation.
    if (!currentPw || !newPw || !confirmPw) {
      setErrorGeneral("All fields are required.");
      return;
    }
    if (newPw !== confirmPw) {
      setErrorConfirm("New password and confirmation do not match.");
      return;
    }
    if (newPw === currentPw) {
      setErrorNew("New password must differ from current.");
      return;
    }

    setBusy(true);
    try {
      await api.changeMasterPassword(currentPw, newPw);
      onChanged();
    } catch (e) {
      const err = e as GuiError;
      if (err.kind === "Locked") {
        onLockedError();
      } else if (err.kind === "WrongPassword") {
        setErrorCurrent("Current password is incorrect.");
      } else {
        setErrorGeneral(`Failed: ${err.message ?? err.kind}`);
      }
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal modal--change-master-pw"
        onClick={(e) => e.stopPropagation()}
      >
        <h2>Change master password</h2>
        <div className="account-form">
          <label>Current:</label>
          <input
            type="password"
            value={currentPw}
            autoFocus
            onChange={(e) => setCurrentPw(e.target.value)}
            disabled={busy}
          />
          {errorCurrent && (
            <>
              <span></span>
              <div className="account-form__error">{errorCurrent}</div>
            </>
          )}

          <label>New:</label>
          <div className="modal__field-row">
            <input
              type="password"
              value={newPw}
              onChange={(e) => setNewPw(e.target.value)}
              disabled={busy}
            />
            <button
              type="button"
              onClick={() => setGenOpen(true)}
              disabled={busy}
              aria-label="Generate password"
              title="Generate password"
              className="modal__gen-btn"
            >
              {"\u{1F3B2}"}
            </button>
          </div>
          {errorNew && (
            <>
              <span></span>
              <div className="account-form__error">{errorNew}</div>
            </>
          )}

          <label>Confirm:</label>
          <input
            type="password"
            value={confirmPw}
            onChange={(e) => setConfirmPw(e.target.value)}
            disabled={busy}
          />
          {errorConfirm && (
            <>
              <span></span>
              <div className="account-form__error">{errorConfirm}</div>
            </>
          )}
        </div>
        {errorGeneral && (
          <div className="account-form__error account-form__error--general">
            {errorGeneral}
          </div>
        )}
        <div className="modal__actions">
          <button onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button onClick={handleChange} disabled={busy}>
            {busy ? "Re-encrypting…" : "Change"}
          </button>
        </div>
        {genOpen && (
          <PasswordGeneratorPopover
            onChoose={(pw) => setNewPw(pw)}
            onClose={() => setGenOpen(false)}
          />
        )}
      </div>
    </div>
  );
}
