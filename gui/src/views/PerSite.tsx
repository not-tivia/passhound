import { useEffect, useState } from "react";
import { api } from "../api";
import PasswordCell from "../components/PasswordCell";
import type { AccountDetail, GuiError } from "../types";

interface PerSiteProps {
  accountId: number;
  onLockedError: () => void;
}

export default function PerSite({ accountId, onLockedError }: PerSiteProps) {
  const [detail, setDetail] = useState<AccountDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setDetail(null);
    setError(null);
    api
      .getAccount(accountId)
      .then(setDetail)
      .catch((e: GuiError) => {
        if (e.kind === "Locked") onLockedError();
        else setError(e.message ?? e.kind);
      });
  }, [accountId, onLockedError]);

  if (error) {
    return <div className="per-site__status per-site__status--error">{error}</div>;
  }
  if (!detail) {
    return <div className="per-site__status">Loading...</div>;
  }

  const current = detail.history.find((h) => h.is_current);
  const past = detail.history.filter((h) => !h.is_current);

  return (
    <div className="per-site">
      <div className="per-site__header">
        <div className="per-site__title">{detail.site_name}</div>
        <div className="per-site__meta">
          {[detail.site_url, detail.site_category, ...detail.site_abbreviations]
            .filter((x): x is string => !!x)
            .join(" · ")}
        </div>
        <div className="per-site__user">
          {detail.username ?? "(no username)"}
        </div>
      </div>

      <div className="per-site__body">
        {current && (
          <>
            <div className="per-site__section-label">Current</div>
            <div className="per-site__entry">
              <PasswordCell historyId={current.id} onLockedError={onLockedError} />
              <div className="per-site__date">{current.created_at.slice(0, 10)}</div>
            </div>
          </>
        )}

        <div className="per-site__section-label">
          History ({past.length})
        </div>
        {past.length === 0 && (
          <div className="per-site__empty">No prior history.</div>
        )}
        {past.map((h) => (
          <div className="per-site__entry per-site__entry--past" key={h.id}>
            <PasswordCell historyId={h.id} onLockedError={onLockedError} />
            <div className="per-site__date">{h.created_at.slice(0, 10)} · {h.source}</div>
          </div>
        ))}
      </div>
    </div>
  );
}
