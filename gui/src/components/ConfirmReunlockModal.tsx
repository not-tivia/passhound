interface ConfirmReunlockModalProps {
  onReunlock: () => void;
}

export default function ConfirmReunlockModal({ onReunlock }: ConfirmReunlockModalProps) {
  return (
    <div className="modal-backdrop">
      <div className="modal modal--confirm-reunlock" onClick={(e) => e.stopPropagation()}>
        <h2>Master password changed</h2>
        <p className="modal__body-text">
          You'll need to re-enter your new password to verify it. Continue?
        </p>
        <div className="modal__actions">
          <button onClick={onReunlock} autoFocus>
            Re-unlock
          </button>
        </div>
      </div>
    </div>
  );
}
