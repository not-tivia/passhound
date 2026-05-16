interface ResetLearningModalProps {
  onClose: () => void;
  onConfirmed: () => void | Promise<void>;
}

export default function ResetLearningModal({ onClose, onConfirmed }: ResetLearningModalProps) {
  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal modal--reset-learning" onClick={(e) => e.stopPropagation()}>
        <h2>Reset learning data</h2>
        <p className="modal__body-text">
          This clears all recovery feedback. The auto-tune resets to neutral
          (every rule's multiplier returns to 1.0). New feedback will accumulate
          from scratch. Continue?
        </p>
        <div className="modal__actions">
          <button onClick={onClose}>Cancel</button>
          <button onClick={() => void onConfirmed()}>Reset</button>
        </div>
      </div>
    </div>
  );
}
