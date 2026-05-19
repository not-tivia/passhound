import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import ColumnMappingTable, {
  ColumnRole,
  validateRoles,
  rolesToMapping,
  mappingToRoles,
} from "../components/ColumnMappingTable";
import ImportPreview from "../components/ImportPreview";
import SkippedRowsPanel from "../components/SkippedRowsPanel";
import { useToast } from "../components/Toast";
import type { GuiError, PreviewResult, RowPatch, PreviewDiagnostic } from "../types";

function isMissingSite(d: PreviewDiagnostic): boolean {
  return !d.parsed.site || d.parsed.site.trim().length === 0;
}
function isMissingPassword(d: PreviewDiagnostic): boolean {
  return !d.parsed.has_password;
}

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
  const [bulkPatch, setBulkPatch] = useState<{ site: string; password: string }>({
    site: "",
    password: "",
  });
  const [rowPatches, setRowPatches] = useState<Map<number, { site?: string; password?: string }>>(
    new Map(),
  );
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const toast = useToast();

  const buildPatches = (): RowPatch[] => {
    if (!preview) return [];
    return preview.diagnostics
      .map((d) => {
        const row = rowPatches.get(d.row) ?? {};
        const sitePatch = isMissingSite(d)
          ? (row.site ?? bulkPatch.site).trim()
          : "";
        const passwordPatch = isMissingPassword(d)
          ? (row.password ?? bulkPatch.password)
          : "";
        return {
          row: d.row,
          site: sitePatch || null,
          password: passwordPatch || null,
        };
      })
      .filter((p) => p.site !== null || p.password !== null);
  };

  // Best-effort: clear any pending import slot if user navigates away.
  useEffect(() => {
    return () => {
      api.cancelPendingImport().catch(() => {});
    };
  }, []);

  const refreshPreviewWithPending = async (
    site: string | null,
    currentRoles: ColumnRole[] | null,
    patches: RowPatch[],
  ) => {
    setError(null);
    try {
      const mapping =
        currentRoles && currentRoles.length > 0
          ? rolesToMapping(currentRoles)
          : null;
      const result = await api.importCsvDryRunWithPending(site, mapping, patches);
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

  // Debounced refresh when site override or roles change (after a file is
  // picked). Patches (bulk + per-row) are NOT in the dep array — they fire
  // a re-parse only on input blur via commitPatches(), so the user can
  // finish typing without the row promoting on every keystroke.
  useEffect(() => {
    if (!hasPending) return;
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      refreshPreviewWithPending(siteOverride || null, roles, buildPatches());
    }, 300);
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hasPending, siteOverride, roles]);

  // Fire a re-parse on patch input blur. Reads current patches from state
  // at call time (not via dep array) so the user's just-typed value is
  // included even though typing didn't trigger the debounce.
  const commitPatches = () => {
    if (!hasPending) return;
    refreshPreviewWithPending(siteOverride || null, roles, buildPatches());
  };

  // Prune stale row patches when diagnostics change (e.g. after re-parse shifts rows).
  useEffect(() => {
    if (!preview) return;
    const liveRows = new Set(preview.diagnostics.map((d) => d.row));
    setRowPatches((prev) => {
      let changed = false;
      const next = new Map<number, { site?: string; password?: string }>();
      for (const [row, patch] of prev) {
        if (liveRows.has(row)) {
          next.set(row, patch);
        } else {
          changed = true;
        }
      }
      return changed ? next : prev;
    });
  }, [preview]);

  const handleBrowse = async () => {
    try {
      const mapping = roles.length > 0 ? rolesToMapping(roles) : null;
      const result = await api.pickAndImportCsvDryRun(siteOverride || null, mapping, []);
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
      // The Tauri command stashed the path BEFORE running the dry-run, so
      // even when the dry-run fails (e.g., "no site column found"), the
      // path is in pending_import_path. Flip hasPending=true so the user
      // can recover by typing into "Apply site to all rows" or adjusting
      // the mapping — the debounced effect will re-run dry-run-with-pending.
      setHasPending(true);
      const raw = err.message ?? err.kind;
      // Friendlier wording for the most common recoverable error.
      const msg = raw.includes("no site column")
        ? "CSV has no site column. Type a site name in \"Apply site to all rows\" above to apply to every row."
        : raw;
      setError(msg);
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
        buildPatches(),
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
            disabled={submitting}
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
            {preview.diagnostics.length > 0 && (
              <SkippedRowsPanel
                diagnostics={preview.diagnostics}
                bulkPatch={bulkPatch}
                rowPatches={rowPatches}
                onBulkChange={(field, value) =>
                  setBulkPatch((prev) => ({ ...prev, [field]: value }))
                }
                onRowChange={(row, field, value) =>
                  setRowPatches((prev) => {
                    const next = new Map(prev);
                    const current = next.get(row) ?? {};
                    next.set(row, { ...current, [field]: value });
                    return next;
                  })
                }
                onCommitPatches={commitPatches}
              />
            )}
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
