import { useState } from "react";
import { api } from "../api";
import { useToast } from "./Toast";
import { useSettings } from "../context/SettingsContext";
import type { CandidateView, GuiError } from "../types";

const MASK = "\u{2022}".repeat(12);

interface CandidateRowProps {
  candidate: CandidateView;
  revealed: boolean;
  tried: boolean;
  onToggleTried: () => void;
  onLockedError: () => void;
}

type FeedbackKind = "none" | "worked" | "didnt-work";

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
  const [feedbackKind, setFeedbackKind] = useState<FeedbackKind>("none");

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

  const handleFeedback = async (worked: boolean) => {
    if (busy) return;
    setBusy(true);
    try {
      const pw = candidate.password;
      await api.recordRecoveryFeedback({
        accountId: null,
        provenance: candidate.provenance.map((r) => r.tag),
        score: candidate.score,
        rank: candidate.rank,
        worked,
        length: pw.length,
        hasDigit: /\d/.test(pw),
        hasSymbol: /[^A-Za-z0-9]/.test(pw),
        hasUpper: /[A-Z]/.test(pw),
        hasLower: /[a-z]/.test(pw),
      });
      setFeedbackKind(worked ? "worked" : "didnt-work");
      toast.show(`Recorded: ${worked ? "worked" : "didn't work"}`);
    } catch (e) {
      const err = e as GuiError;
      if (err.kind === "Locked") onLockedError();
      else toast.show(`Failed to record: ${err.message ?? err.kind}`);
    } finally {
      setBusy(false);
    }
  };

  const provenanceText = candidate.provenance.length === 0
    ? "(no rules)"
    : `${candidate.provenance.map((r) => r.tag).join("+")}: ${candidate.provenance.map((r) => r.name).join(" + ")}`;

  return (
    <div className={`candidate-row${tried ? " candidate-row--tried" : ""}${feedbackKind !== "none" ? " candidate-row--feedback" : ""}`}>
      <span className="candidate-row__rank">#{candidate.rank}</span>
      <span className="candidate-row__score">{candidate.score.toFixed(2)}</span>
      <span className="candidate-row__pw">{revealed ? candidate.password : MASK}</span>
      <span className="candidate-row__why" title={provenanceText}>{provenanceText}</span>
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
          {tried ? "\u{2713}" : "\u{25EF}"}
        </button>
        <button
          className={`candidate-row__btn candidate-row__btn--worked${feedbackKind === "worked" ? " candidate-row__btn--filled" : ""}`}
          onClick={() => handleFeedback(true)}
          disabled={busy}
          aria-label="Mark as worked"
          title="Mark as worked"
        >
          {"\u{2713}"}
        </button>
        <button
          className={`candidate-row__btn candidate-row__btn--didnt-work${feedbackKind === "didnt-work" ? " candidate-row__btn--filled" : ""}`}
          onClick={() => handleFeedback(false)}
          disabled={busy}
          aria-label="Mark as didn't work"
          title="Mark as didn't work"
        >
          {"\u{2715}"}
        </button>
      </span>
    </div>
  );
}
