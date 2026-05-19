import type { PreviewResult } from "../types";

interface ImportPreviewProps {
  result: PreviewResult;
}

export default function ImportPreview({ result }: ImportPreviewProps) {
  const { counts, sample_rows, diagnostics } = result;
  return (
    <div className="import-preview">
      <div className="import-preview__summary">
        <span className="import-preview__num">{counts.new} new</span>,{" "}
        <span>{counts.duplicates} duplicates</span>,{" "}
        <span>{counts.merges} merges</span>
        {counts.errors > 0 && (
          <span className="import-preview__errors">
            , {counts.errors} skipped
          </span>
        )}
      </div>
      {sample_rows.length > 0 && (
        <table className="import-preview__rows">
          <thead>
            <tr>
              <th>SITE</th>
              <th>USERNAME</th>
              <th>DISPLAY</th>
              <th>PASSWORD</th>
              <th>NOTES</th>
            </tr>
          </thead>
          <tbody>
            {sample_rows.map((r, i) => (
              <tr key={i}>
                <td>{r.site}</td>
                <td>{r.username ?? <span className="muted">—</span>}</td>
                <td>{r.display_name ?? <span className="muted">—</span>}</td>
                <td>
                  <span className="import-preview__pwd">
                    {"•".repeat(Math.min(r.password_length, 12))}
                  </span>{" "}
                  <span className="muted">({r.password_length})</span>
                </td>
                <td className="import-preview__notes">
                  {r.notes ?? <span className="muted">—</span>}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {diagnostics.length > 0 && (
        <div className="import-preview__diagnostics">
          <div className="import-preview__diag-label">Skipped:</div>
          <ul>
            {diagnostics.slice(0, 5).map((d, i) => (
              <li key={i}>{d.reason}</li>
            ))}
            {diagnostics.length > 5 && (
              <li className="muted">…and {diagnostics.length - 5} more</li>
            )}
          </ul>
        </div>
      )}
    </div>
  );
}
