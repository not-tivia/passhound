import type { CandidateView } from "../types";
import CandidateRow from "./CandidateRow";

interface RecoveryResultsProps {
  candidates: CandidateView[] | null;
  loading: boolean;
  error: { kind: "EmptyVault" } | { kind: "Other"; message: string } | null;
  revealAll: boolean;
  onToggleReveal: () => void;
  triedIds: Set<number>;
  onToggleTried: (rank: number) => void;
  onNavigateToImport: () => void;
  onLockedError: () => void;
}

export default function RecoveryResults({
  candidates,
  loading,
  error,
  revealAll,
  onToggleReveal,
  triedIds,
  onToggleTried,
  onNavigateToImport,
  onLockedError,
}: RecoveryResultsProps) {
  return (
    <div className="recovery-results">
      <div className="recovery-results__header">
        <span className="recovery-results__status">
          {loading
            ? "Running…"
            : candidates === null
              ? "No run yet"
              : `${candidates.length} candidates`}
        </span>
        <button
          className="recovery-results__reveal"
          onClick={onToggleReveal}
          disabled={candidates === null || candidates.length === 0}
        >
          {revealAll ? "Hide candidates" : "Reveal candidates"}
        </button>
      </div>

      <div className="recovery-results__body">
        {error?.kind === "EmptyVault" && (
          <div className="recovery-results__empty">
            <p>Vault has no history yet. Import some accounts first.</p>
            <button
              className="recovery-results__link-btn"
              onClick={onNavigateToImport}
            >
              Go to Import
            </button>
          </div>
        )}

        {error?.kind === "Other" && (
          <div className="recovery-results__error">
            Recovery failed: {error.message ?? "unknown error"}
          </div>
        )}

        {!error && candidates === null && !loading && (
          <div className="recovery-results__empty">
            Set filters and click Run to begin.
          </div>
        )}

        {!error && candidates !== null && candidates.length === 0 && !loading && (
          <div className="recovery-results__empty">
            No candidates matched these filters. Try relaxing length / symbol / digit constraints.
          </div>
        )}

        {!error && candidates !== null && candidates.length > 0 && (
          <div className="recovery-results__table">
            <div className="candidate-row candidate-row--header">
              <span className="candidate-row__rank">#</span>
              <span className="candidate-row__score">SCORE</span>
              <span className="candidate-row__pw">CANDIDATE</span>
              <span className="candidate-row__why">WHY</span>
              <span className="candidate-row__actions" />
            </div>
            {candidates.map((c) => (
              <CandidateRow
                key={c.rank}
                candidate={c}
                revealed={revealAll}
                tried={triedIds.has(c.rank)}
                onToggleTried={() => onToggleTried(c.rank)}
                onLockedError={onLockedError}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
