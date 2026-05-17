import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import AccountFormModal from "../components/AccountFormModal";
import AddPasswordInput from "../components/AddPasswordInput";
import AttachmentsSection from "../components/AttachmentsSection";
import EditSiteModal from "../components/EditSiteModal";
import PasswordCell from "../components/PasswordCell";
import TagChip from "../components/TagChip";
import TagPicker from "../components/TagPicker";
import { useToast } from "../components/Toast";
import type { AccountDetail, GuiError, TagSummary } from "../types";

interface PerSiteProps {
  accountId: number;
  onLockedError: () => void;
  onAccountDeleted: () => void;
  onRecoverAccount: (siteName: string, accountLabel: string | null) => void;
  onSiteUpdated: () => void;
}

export default function PerSite({ accountId, onLockedError, onAccountDeleted, onRecoverAccount, onSiteUpdated }: PerSiteProps) {
  const [detail, setDetail] = useState<AccountDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState(false);
  const [editSiteOpen, setEditSiteOpen] = useState(false);
  const [addingPassword, setAddingPassword] = useState(false);
  const toast = useToast();

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
            className="per-site__recover-account"
            onClick={() =>
              onRecoverAccount(
                detail.site_name,
                detail.username ?? detail.display_name ?? null,
              )
            }
            title="Open Recovery view pre-filled with this account"
          >
            Recover this password
          </button>
          <button
            className="per-site__edit-site"
            onClick={() => setEditSiteOpen(true)}
            title="Edit site metadata"
          >
            Edit site
          </button>
          <button
            className="per-site__edit-account"
            onClick={() => setEditing(true)}
            title="Edit account metadata"
          >
            Edit
          </button>
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
        {detail.site_notes && detail.site_notes.trim() && (
          <div className="per-site__site-notes">{detail.site_notes}</div>
        )}
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
        <div className="per-site__section-label">Current</div>
        {current ? (
          <div className="per-site__entry">
            <PasswordCell
              historyId={current.id}
              onLockedError={onLockedError}
              onDelete={loadDetail}
              onEditCurrent={async (newPlaintext) => {
                try {
                  await api.addPassword(accountId, newPlaintext, "manual-edit");
                  toast.show("Password updated. Old value moved to history.");
                  loadDetail();
                } catch (e) {
                  const err = e as GuiError;
                  if (err.kind === "Locked") onLockedError();
                  else toast.show(`Update failed: ${err.message ?? err.kind}`);
                  throw e;
                }
              }}
            />
            <div className="per-site__date">{current.created_at.slice(0, 10)}</div>
          </div>
        ) : addingPassword ? (
          <AddPasswordInput
            accountId={accountId}
            onSave={() => { setAddingPassword(false); loadDetail(); }}
            onCancel={() => setAddingPassword(false)}
            onLockedError={onLockedError}
          />
        ) : (
          <button className="account-form__add-btn" onClick={() => setAddingPassword(true)}>
            + Add password
          </button>
        )}

        <div className="per-site__section-label">
          History ({past.length})
        </div>
        {past.length === 0 && (
          <div className="per-site__empty">No prior history.</div>
        )}
        {past.map((h) => (
          <div className="per-site__entry per-site__entry--past" key={h.id}>
            <PasswordCell
              historyId={h.id}
              onLockedError={onLockedError}
              onDelete={loadDetail}
              onPromote={async () => {
                try {
                  await api.promotePassword(h.id);
                  loadDetail();
                } catch (e) {
                  const err = e as GuiError;
                  if (err.kind === "Locked") onLockedError();
                  else setError(err.message ?? err.kind);
                }
              }}
            />
            <div className="per-site__date">{h.created_at.slice(0, 10)} · {h.source}</div>
          </div>
        ))}
        <AttachmentsSection accountId={detail.id} />
      </div>
      {editing && (
        <AccountFormModal
          mode="edit"
          initial={{
            id: detail.id,
            site_id: detail.site_id,
            site_name: detail.site_name,
            username: detail.username,
            display_name: detail.display_name,
            alias: detail.alias,
            notes: detail.notes,
          }}
          onClose={() => setEditing(false)}
          onSaved={() => { setEditing(false); loadDetail(); }}
          onLockedError={onLockedError}
        />
      )}
      {editSiteOpen && (
        <EditSiteModal
          detail={detail}
          onClose={() => setEditSiteOpen(false)}
          onSaved={() => {
            loadDetail();
            onSiteUpdated();
          }}
          onLockedError={onLockedError}
        />
      )}
    </div>
  );
}
