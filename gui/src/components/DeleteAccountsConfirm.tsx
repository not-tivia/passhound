import { useState } from "react";

interface DeleteAccountsConfirmProps {
  count: number;
  onConfirm: () => void;
  onCancel: () => void;
}

const HARD_THRESHOLD = 20;

export default function DeleteAccountsConfirm({ count, onConfirm, onCancel }: DeleteAccountsConfirmProps) {
  const [typed, setTyped] = useState("");
  const requiresType = count >= HARD_THRESHOLD;
  const canConfirm = !requiresType || typed === "DELETE";

  return (
    <div className="modal-backdrop" onClick={onCancel}>
      <div className="modal modal--confirm" onClick={(e) => e.stopPropagation()}>
        <h2>Delete {count} account{count === 1 ? "" : "s"}?</h2>
        <p>All passwords and attachments will be deleted. This cannot be undone.</p>
        {requiresType && (
          <>
            <p className="modal__warn">Type <strong>DELETE</strong> to confirm:</p>
            <input
              autoFocus
              value={typed}
              onChange={(e) => setTyped(e.target.value)}
              className="modal__input"
            />
          </>
        )}
        <div className="modal__actions">
          <button onClick={onCancel}>Cancel</button>
          <button
            disabled={!canConfirm}
            onClick={onConfirm}
            className="modal__btn--danger"
          >
            Delete {count} account{count === 1 ? "" : "s"}
          </button>
        </div>
      </div>
    </div>
  );
}
