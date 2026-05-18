import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import ColumnMappingTable, {
  ColumnRole,
  validateRoles,
  rolesToMapping,
  mappingToRoles,
} from "../components/ColumnMappingTable";
import ImportPreview from "../components/ImportPreview";
import { useToast } from "../components/Toast";
import type { GuiError, PreviewResult } from "../types";

interface ImportProps {
  onDone: () => void;
  onLockedError: () => void;
}

export default function Import({ onDone, onLockedError }: ImportProps) {
  const [hasPending, setHasPending] = useState(false);
  const [siteOverride, setSiteOverride] = useState("");
  const [roles, setRoles] = useState<ColumnRole[]>([]);
  const [preview, setPreview] = useState<PreviewResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const toast = useToast();

  // Best-effort: clear any pending import slot if user navigates away.
  useEffect(() => {
    return () => {
      api.cancelPendingImport().catch(() => {});
    };
  }, []);

  const refreshPreviewWithPending = async (
    site: string | null,
    currentRoles: ColumnRole[] | null,
  ) => {
    setError(null);
    try {
      const mapping =
        currentRoles && currentRoles.length > 0
          ? rolesToMapping(currentRoles)
          : null;
      const result = await api.importCsvDryRunWithPending(site, mapping);
      setPreview(result);
      // Sync roles to the effective mapping on the first preview only —
      // don't clobber the user's in-progress edits.
      if (!currentRoles || currentRoles.length === 0) {
        setRoles(mappingToRoles(result.headers, result.effective_mapping));
      }
    } catch (e) {
      const err = e as GuiError;
      if (err.kind === "Locked") {
        onLockedError();
        return;
      }
      setError(err.message ?? err.kind);
      setPreview(null);
    }
  };

  // Debounced refresh when site override or roles change (after a file is picked).
  useEffect(() => {
    if (!hasPending) return;
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      refreshPreviewWithPending(siteOverride || null, roles);
    }, 300);
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hasPending, siteOverride, roles]);

  const handleBrowse = async () => {
    try {
      const mapping = roles.length > 0 ? rolesToMapping(roles) : null;
      const result = await api.pickAndImportCsvDryRun(siteOverride || null, mapping);
      if (result === null) {
        // User cancelled the dialog — do nothing.
        return;
      }
      setHasPending(true);
      setRoles([]);  // Reset; result already has auto-detected mapping.
      setPreview(result);
      setError(null);
      setRoles(mappingToRoles(result.headers, result.effective_mapping));
    } catch (e) {
      const err = e as GuiError;
      if (err.kind === "Locked") {
        onLockedError();
        return;
      }
      setError(err.message ?? `dialog: ${err.kind}`);
    }
  };

  const handleImport = async () => {
    if (!hasPending || !preview) return;
    const validationErr = validateRoles(roles);
    if (validationErr) {
      setError(validationErr);
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      const mapping = rolesToMapping(roles);
      const result = await api.importCsvCommitPending(
        siteOverride || null,
        mapping,
      );
      toast.show(`Imported ${result.counts.new} entries`);
      onDone();
    } catch (e) {
      const err = e as GuiError;
      if (err.kind === "Locked") {
        onLockedError();
        return;
      }
      setError(err.message ?? err.kind);
    } finally {
      setSubmitting(false);
    }
  };

  const handleCancel = () => {
    api.cancelPendingImport().catch(() => {});
    onDone();
  };

  const validationErr = roles.length > 0 ? validateRoles(roles) : null;
  const importDisabled = !preview || submitting || validationErr !== null;

  return (
    <div className="import-view">
      <div className="import-view__header">Import CSV</div>
      <div className="import-view__body">
        <div className="import-view__row">
          <span className="import-view__label">File:</span>
          <input
            className="import-view__path"
            placeholder="No file selected"
            value={preview ? "(file selected)" : ""}
            readOnly
          />
          <button
            className="import-view__btn"
            onClick={handleBrowse}
            disabled={submitting}
          >
            Browse…
          </button>
        </div>
        <div className="import-view__row">
          <span className="import-view__label">Apply site to all rows:</span>
          <input
            className="import-view__site"
            placeholder="(optional — use only for per-site CSVs)"
            value={siteOverride}
            onChange={(e) => setSiteOverride(e.target.value)}
            disabled={!hasPending || submitting}
          />
        </div>

        {error && <div className="import-view__error">{error}</div>}

        {preview && (
          <>
            <ColumnMappingTable
              headers={preview.headers}
              roles={roles}
              onChange={(i, r) => {
                const next = roles.slice();
                next[i] = r;
                setRoles(next);
              }}
            />
            <ImportPreview result={preview} />
          </>
        )}

        {validationErr && roles.length > 0 && (
          <div className="import-view__error">{validationErr}</div>
        )}
      </div>

      <div className="import-view__footer">
        <button
          className="import-view__btn import-view__btn--ghost"
          onClick={handleCancel}
          disabled={submitting}
        >
          Cancel
        </button>
        <button
          className="import-view__btn import-view__btn--primary"
          onClick={handleImport}
          disabled={importDisabled}
          title={validationErr ?? undefined}
        >
          {submitting
            ? "Importing…"
            : preview
              ? `Import ${preview.counts.new} entries`
              : "Import"}
        </button>
      </div>
    </div>
  );
}
