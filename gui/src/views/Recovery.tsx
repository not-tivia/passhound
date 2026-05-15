import { useEffect, useRef, useState } from "react";
import RecoveryFilters from "../components/RecoveryFilters";
import RecoveryResults from "../components/RecoveryResults";
import { useSettings } from "../context/SettingsContext";
import { api } from "../api";
import type { CandidateView, GuiError, RecoverFilters } from "../types";

interface RecoveryInitial {
  site?: string;
  account?: string;
}

interface RecoveryProps {
  initial?: RecoveryInitial;
  onLockedError: () => void;
  onNavigateToImport: () => void;
}

const DEFAULT_FILTERS: RecoverFilters = {
  site: null,
  account: null,
  era: null,
  hint: null,
  limit: 500,
  minLength: null,
  requireSymbol: false,
  requireDigit: false,
};

type ResultsError =
  | { kind: "EmptyVault" }
  | { kind: "EraNotFound"; eraName: string }
  | { kind: "Other"; message: string };

export default function Recovery({
  initial,
  onLockedError,
  onNavigateToImport,
}: RecoveryProps) {
  const [filters, setFilters] = useState<RecoverFilters>({
    ...DEFAULT_FILTERS,
    site: initial?.site ?? null,
    account: initial?.account ?? null,
  });
  const { settings } = useSettings();
  const [candidates, setCandidates] = useState<CandidateView[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<ResultsError | null>(null);
  // useState(initial) only uses initial on first render — subsequent changes to
  // settings.default_reveal do not override the user's in-view toggle.
  const [revealAll, setRevealAll] = useState(settings.default_reveal);
  const [triedIds, setTriedIds] = useState<Set<number>>(new Set());

  const hasRunOnce = useRef<boolean>(false);

  const run = async (next: RecoverFilters) => {
    setLoading(true);
    setError(null);
    try {
      const result = await api.recoverCandidates(next);
      setCandidates(result);
      setTriedIds(new Set()); // Fresh run = fresh tried state.
      hasRunOnce.current = true;
    } catch (e) {
      const err = e as GuiError;
      if (err.kind === "Locked") {
        onLockedError();
        return;
      }
      if (err.kind === "EmptyVault") {
        setError({ kind: "EmptyVault" });
        setCandidates(null);
        return;
      }
      if (err.kind === "EraNotFound") {
        setError({ kind: "EraNotFound", eraName: err.message ?? "" });
        setCandidates(null);
        return;
      }
      setError({ kind: "Other", message: err.message ?? err.kind });
      setCandidates(null);
    } finally {
      setLoading(false);
    }
  };

  // Hybrid auto-rerun: fires on hint/limit changes ONLY after the user has
  // explicitly clicked Run at least once. 300ms debounce.
  useEffect(() => {
    if (!hasRunOnce.current) return;
    const t = setTimeout(() => {
      run(filters);
    }, 300);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filters.hint, filters.limit]);

  const eraError = error?.kind === "EraNotFound" ? error.eraName : null;

  return (
    <div className="recovery">
      <div className="recovery-filters-wrap">
        <RecoveryFilters
          filters={filters}
          onChange={setFilters}
          onRun={() => run(filters)}
          running={loading}
          onLockedError={onLockedError}
        />
        {eraError && (
          <div className="recovery-filters__era-error">
            No era named "{eraError}".
          </div>
        )}
      </div>
      <RecoveryResults
        candidates={candidates}
        loading={loading}
        error={
          error?.kind === "EmptyVault"
            ? { kind: "EmptyVault" }
            : error?.kind === "Other"
              ? { kind: "Other", message: error.message }
              : null
        }
        revealAll={revealAll}
        onToggleReveal={() => setRevealAll((v) => !v)}
        triedIds={triedIds}
        onToggleTried={(rank) =>
          setTriedIds((prev) => {
            const next = new Set(prev);
            if (next.has(rank)) next.delete(rank);
            else next.add(rank);
            return next;
          })
        }
        onNavigateToImport={onNavigateToImport}
        onLockedError={onLockedError}
      />
    </div>
  );
}
