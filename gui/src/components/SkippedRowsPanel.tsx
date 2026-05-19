import type { PreviewDiagnostic } from "../types";

function isMissingSite(d: PreviewDiagnostic): boolean {
  return !d.parsed.site || d.parsed.site.trim().length === 0;
}

function isMissingPassword(d: PreviewDiagnostic): boolean {
  return !d.parsed.has_password;
}

type SkippedRowsPanelProps = {
  diagnostics: PreviewDiagnostic[];
  bulkPatch: { site: string; password: string };
  rowPatches: Map<number, { site?: string; password?: string }>;
  onBulkChange: (field: "site" | "password", value: string) => void;
  onRowChange: (row: number, field: "site" | "password", value: string) => void;
  /** Called when a patch input loses focus — triggers a re-parse with the
   * current patches. Letting onChange auto-fire would re-parse on every
   * keystroke and promote partially-typed values, deleting the row the
   * user is still editing. */
  onCommitPatches: () => void;
};

export default function SkippedRowsPanel({
  diagnostics,
  bulkPatch,
  rowPatches,
  onBulkChange,
  onRowChange,
  onCommitPatches,
}: SkippedRowsPanelProps) {
  if (diagnostics.length === 0) return null;

  const missingSiteCount = diagnostics.filter(isMissingSite).length;
  const missingPasswordCount = diagnostics.filter(isMissingPassword).length;

  return (
    <div className="skipped-rows-panel">
      <h4 className="skipped-rows-panel__title">
        Skipped rows ({diagnostics.length})
      </h4>

      {missingSiteCount > 0 && (
        <div className="skipped-rows-panel__bulk">
          <label>
            <span>
              Apply site to all {missingSiteCount} row
              {missingSiteCount === 1 ? "" : "s"} missing site:
            </span>
            <input
              type="text"
              value={bulkPatch.site}
              onChange={(e) => onBulkChange("site", e.target.value)}
              onBlur={onCommitPatches}
              placeholder="(site name)"
            />
          </label>
        </div>
      )}

      {missingPasswordCount > 0 && (
        <div className="skipped-rows-panel__bulk">
          <label>
            <span>
              Apply password to all {missingPasswordCount} row
              {missingPasswordCount === 1 ? "" : "s"} missing password:
            </span>
            <input
              type="text"
              value={bulkPatch.password}
              onChange={(e) => onBulkChange("password", e.target.value)}
              onBlur={onCommitPatches}
              placeholder="(password)"
            />
          </label>
        </div>
      )}

      <table className="skipped-rows-panel__table">
        <thead>
          <tr>
            <th>Row</th>
            <th>Raw</th>
            <th>Reason</th>
            <th>Fix</th>
          </tr>
        </thead>
        <tbody>
          {diagnostics.map((d) => {
            const rowPatch = rowPatches.get(d.row) ?? {};
            const showSiteInput = isMissingSite(d);
            const showPasswordInput = isMissingPassword(d);
            const siteValue = rowPatch.site ?? bulkPatch.site;
            const passwordValue = rowPatch.password ?? bulkPatch.password;
            return (
              <tr key={d.row}>
                <td>{d.row}</td>
                <td className="skipped-rows-panel__raw">{d.raw}</td>
                <td>{d.reason}</td>
                <td className="skipped-rows-panel__fix">
                  {showSiteInput && (
                    <label>
                      site:
                      <input
                        type="text"
                        value={siteValue}
                        onChange={(e) =>
                          onRowChange(d.row, "site", e.target.value)
                        }
                        onBlur={onCommitPatches}
                      />
                    </label>
                  )}
                  {showPasswordInput && (
                    <label>
                      password:
                      <input
                        type="text"
                        value={passwordValue}
                        onChange={(e) =>
                          onRowChange(d.row, "password", e.target.value)
                        }
                        onBlur={onCommitPatches}
                      />
                    </label>
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
