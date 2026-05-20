import { useEffect, useState } from "react";
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
  const [rowRevealed, setRowRevealed] = useState(revealed);
  const [breakdownExpanded, setBreakdownExpanded] = useState(false);
  const [revealedText, setRevealedText] = useState<string | null>(null);

  // Keep local rowRevealed in sync with the prop (revealAll toggle).
  useEffect(() => {
    setRowRevealed(revealed);
  }, [revealed]);

  // Auto-mask after reveal_clear_seconds when non-zero.
  useEffect(() => {
    if (!rowRevealed) return;
    if (settings.reveal_clear_seconds === 0) return;
    const handle = setTimeout(
      () => setRowRevealed(false),
      settings.reveal_clear_seconds * 1000,
    );
    return () => clearTimeout(handle);
  }, [rowRevealed, settings.reveal_clear_seconds]);

  // Lazy-fetch plaintext when rowRevealed flips true; clear when false.
  useEffect(() => {
    let cancelled = false;
    if (rowRevealed) {
      api.revealCandidate(candidate.rank)
        .then((pt) => { if (!cancelled) setRevealedText(pt); })
        .catch(() => { if (!cancelled) setRevealedText(null); });
    } else {
      setRevealedText(null);
    }
    return () => { cancelled = true; };
  }, [rowRevealed, candidate.rank]);

  const handleCopy = async () => {
    if (busy) return;
    setBusy(true);
    try {
      await api.copyCandidate(candidate.rank);
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
      // Char-class learning needs the plaintext. If the user hasn't revealed
      // this row, fetch it from the cache so the feedback record carries
      // accurate hasDigit/hasSymbol/hasUpper/hasLower flags.
      const pw = revealedText ?? (await api.revealCandidate(candidate.rank));
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

  const bd = candidate.breakdown;

  return (
    <>
      <div className={`candidate-row${tried ? " candidate-row--tried" : ""}${feedbackKind !== "none" ? " candidate-row--feedback" : ""}`}>
        <span className="candidate-row__rank">
          {bd && (
            <button
              className="candidate-row__chevron"
              onClick={() => setBreakdownExpanded((v) => !v)}
              aria-label={breakdownExpanded ? "Collapse breakdown" : "Expand breakdown"}
              title={breakdownExpanded ? "Collapse score breakdown" : "Expand score breakdown"}
            >
              {breakdownExpanded ? "▾" : "▸"}
            </button>
          )}
          #{candidate.rank}
        </span>
        <span className="candidate-row__score">{candidate.score.toFixed(2)}</span>
        <span className="candidate-row__pw">{rowRevealed && revealedText !== null ? revealedText : MASK}</span>
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
      {breakdownExpanded && bd && (
        <div className="candidate-row__breakdown-row">
          <table className="candidate-row__breakdown">
            <tbody>
              {(
                [
                  ["site",         bd.site,         bd.site_weighted],
                  ["hint",         bd.hint,         bd.hint_weighted],
                  ["freq",         bd.freq,         bd.freq_weighted],
                  ["fav",          bd.fav,          bd.fav_weighted],
                  ["length",       bd.len,          bd.len_weighted],
                  ["orig_casing",  bd.orig_casing,  bd.orig_casing_weighted],
                  ["clean_pat",    bd.clean_pattern, bd.clean_pattern_weighted],
                  ["history_seed", bd.history_seed, bd.history_seed_weighted],
                ] as [string, number, number][]
              ).map(([name, raw, weighted]) => (
                <tr key={name}>
                  <td>{name}</td>
                  <td>{raw.toFixed(2)}</td>
                  <td>=</td>
                  <td>{weighted.toFixed(2)}</td>
                </tr>
              ))}
              <tr>
                <td colSpan={3}>multiplier</td>
                <td>&times;{bd.multiplier.toFixed(2)}</td>
              </tr>
              <tr className="candidate-row__breakdown-total">
                <td colSpan={3}>total</td>
                <td>{bd.total.toFixed(3)}</td>
              </tr>
            </tbody>
          </table>
        </div>
      )}
    </>
  );
}
