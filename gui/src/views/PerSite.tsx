import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import AttachmentsSection from "../components/AttachmentsSection";
import PasswordCell from "../components/PasswordCell";
import TagChip from "../components/TagChip";
import TagPicker from "../components/TagPicker";
import type { AccountDetail, GuiError, TagSummary } from "../types";

interface PerSiteProps {
  accountId: number;
  onLockedError: () => void;
  onAccountDeleted: () => void;
}

export default function PerSite({ accountId, onLockedError, onAccountDeleted }: PerSiteProps) {
  const [detail, setDetail] = useState<AccountDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);

  const loadDetail = useCallback(() => {
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

  useEffect(() => {
    loadDetail();
  }, [loadDetail]);

  const handleDeleteAccount = async () => {
    if (!detail) return;
    const label = detail.username ?? detail.display_name ?? "(no label)";
    if (
      !confirm(
        `Delete account "${detail.site_name}" (${label})?\n\nAll passwords and attachments will be deleted. This cannot be undone.`
      )
    )
      return;
    try {
      await api.deleteAccount(accountId);
      onAccountDeleted();
    } catch (e) {
      const err = e as GuiError;
      if (err.kind === "Locked") onLockedError();
      else setError(err.message ?? err.kind);
    }
  };

  const handleAddTag = async (chosen: TagSummary | { name: string; isNew: true }) => {
    setAdding(false);
    try {
      let tagId: number;
      if ("isNew" in chosen) {
        const created = await api.createTag(chosen.name);
        tagId = created.id;
      } else {
        tagId = chosen.id;
      }
      await api.assignTag(accountId, tagId);
      loadDetail();
    } catch (e) {
      const err = e as GuiError;
      if (err.kind === "Locked") onLockedError();
      else setError(err.message ?? err.kind);
    }
  };

  const handleRemoveTag = async (tagId: number) => {
    try {
      await api.unassignTag(accountId, tagId);
      loadDetail();
    } catch (e) {
      const err = e as GuiError;
      if (err.kind === "Locked") onLockedError();
      else setError(err.message ?? err.kind);
    }
  };

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
        <div className="per-site__title-row">
          <div className="per-site__title">{detail.site_name}</div>
          <button
            className="per-site__delete-account"
            onClick={handleDeleteAccount}
            title="Delete this account permanently"
          >
            Delete account
          </button>
        </div>
        <div className="per-site__tags-row">
          {detail.tags?.map((t) => (
            <TagChip key={t.id} tag={t} onRemove={() => handleRemoveTag(t.id)} />
          ))}
          {!adding ? (
            <button className="per-site__add-tag" onClick={() => setAdding(true)}>+ Add tag</button>
          ) : (
            <TagPicker
              onSelect={handleAddTag}
              onCancel={() => setAdding(false)}
              onLockedError={onLockedError}
              placeholder="tag name…"
            />
          )}
        </div>
        <div className="per-site__meta">
          {[detail.site_url, detail.site_category, ...detail.site_abbreviations]
            .filter((x): x is string => !!x)
            .join(" · ")}
        </div>
        <div className="per-site__user">
          {detail.username ?? "(no username)"}
        </div>
        {detail.display_name && (
          <div className="per-site__display-name">
            <span className="per-site__field-label">display</span>{" "}
            {detail.display_name}
          </div>
        )}
      </div>

      <div className="per-site__body">
        {current && (
          <>
            <div className="per-site__section-label">Current</div>
            <div className="per-site__entry">
              <PasswordCell historyId={current.id} onLockedError={onLockedError} onDelete={loadDetail} />
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
            <PasswordCell historyId={h.id} onLockedError={onLockedError} onDelete={loadDetail} />
            <div className="per-site__date">{h.created_at.slice(0, 10)} · {h.source}</div>
          </div>
        ))}
        <AttachmentsSection accountId={detail.id} />
      </div>
    </div>
  );
}
