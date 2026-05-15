import { useState } from "react";
import { api } from "../api";
import { useToast } from "./Toast";
import { useSettings } from "../context/SettingsContext";
import type { CandidateView, GuiError } from "../types";

const MASK = "•".repeat(12);

interface CandidateRowProps {
  candidate: CandidateView;
  revealed: boolean;
  tried: boolean;
  onToggleTried: () => void;
  onLockedError: () => void;
}

export default function CandidateRow({
  candidate,
  revealed,
  tried,
  onToggleTried,
  onLockedError,
}: CandidateRowProps) {
  const toast = useToast();
  const { settings } = useSettings();
  const [busy, setBusy] = useState(false);

  const handleCopy = async () => {
    if (busy) return;
    setBusy(true);
    try {
      await api.copyToClipboardWithAutoClear(candidate.password, settings.clipboard_clear_seconds);
      toast.show("Copied");
    } catch (e) {
      const err = e as GuiError;
      if (err.kind === "Locked") onLockedError();
      else toast.show(`Copy failed: ${err.message ?? err.kind}`);
    } finally {
      setBusy(false);
    }
  };

  const provenanceText = candidate.provenance.length === 0
    ? "(no rules)"
    : `${candidate.provenance.map((r) => r.tag).join("+")}: ${candidate.provenance.map((r) => r.name).join(" + ")}`;

  return (
    <div className={`candidate-row${tried ? " candidate-row--tried" : ""}`}>
      <span className="candidate-row__rank">#{candidate.rank}</span>
      <span className="candidate-row__score">{candidate.score.toFixed(2)}</span>
      <span className="candidate-row__pw">
        {revealed ? candidate.password : MASK}
      </span>
      <span className="candidate-row__why" title={provenanceText}>
        {provenanceText}
      </span>
      <span className="candidate-row__actions">
        <button
          className="candidate-row__btn"
          onClick={handleCopy}
          disabled={busy}
          aria-label="Copy candidate"
          title="Copy to clipboard"
        >
          {"\u{1F4CB}"}
        </button>
        <button
          className="candidate-row__btn candidate-row__btn--tried"
          onClick={onToggleTried}
          aria-label={tried ? "Unmark tried" : "Mark as tried"}
          title={tried ? "Unmark tried" : "Mark as tried"}
        >
          {tried ? "✓" : "◯"}
        </button>
      </span>
    </div>
  );
}
