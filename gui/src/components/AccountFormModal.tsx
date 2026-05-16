import { useState } from "react";
import { api } from "../api";
import SitePicker from "./SitePicker";
import PasswordGeneratorPopover from "./PasswordGeneratorPopover";
import type { SiteSummary, GuiError } from "../types";

interface AccountInitial {
  id?: number;
  site_id?: number;
  site_name?: string;
  username?: string | null;
  display_name?: string | null;
  alias?: string | null;
  notes?: string | null;
}

interface AccountFormModalProps {
  mode: "add" | "edit";
  initial?: AccountInitial;
  onClose: () => void;
  onSaved: (accountId: number) => void;
  onLockedError: () => void;
}

export default function AccountFormModal({ mode, initial, onClose, onSaved, onLockedError }: AccountFormModalProps) {
  const [sitePicked, setSitePicked] = useState<SiteSummary | { name: string; isNew: true } | null>(
    initial?.site_id && initial?.site_name
      ? { id: initial.site_id, name: initial.site_name }
      : null
  );
  const [pickingSite, setPickingSite] = useState(false);
  const [username, setUsername] = useState(initial?.username ?? "");
  const [displayName, setDisplayName] = useState(initial?.display_name ?? "");
  const [alias, setAlias] = useState(initial?.alias ?? "");
  const [notes, setNotes] = useState(initial?.notes ?? "");
  const [initialPassword, setInitialPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [genOpen, setGenOpen] = useState(false);

  const canSave = mode === "edit" || !!sitePicked;

  const handleSave = async () => {
    if (!canSave || busy) return;
    setBusy(true);
    setError(null);
    try {
      if (mode === "add") {
        let siteId: number;
        if (sitePicked && "isNew" in sitePicked) {
          const created = await api.findOrCreateSite(sitePicked.name);
          siteId = created.id;
        } else if (sitePicked) {
          siteId = (sitePicked as SiteSummary).id;
        } else {
          throw new Error("Site is required");
        }
        const id = await api.addAccount({
          siteId,
          username: username || null,
          displayName: displayName || null,
          alias: alias || null,
          notes: notes || null,
          initialPassword: initialPassword || null,
        });
        onSaved(id);
      } else {
        if (!initial?.id) throw new Error("missing account id");
        await api.updateAccount(initial.id, {
          username: username || null,
          displayName: displayName || null,
          alias: alias || null,
          notes: notes || null,
        });
        onSaved(initial.id);
      }
    } catch (e) {
      const err = e as GuiError;
      if (err.kind === "Locked") onLockedError();
      else setError(err.message ?? err.kind ?? String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal modal--account-form" onClick={(e) => e.stopPropagation()}>
        <h2>{mode === "add" ? "Add account" : "Edit account"}</h2>
        <div className="account-form">
          <label>Site:</label>
          {mode === "add" ? (
            pickingSite ? (
              <SitePicker
                onSelect={(s) => { setSitePicked(s); setPickingSite(false); }}
                onCancel={() => setPickingSite(false)}
                onLockedError={onLockedError}
                placeholder="site name…"
              />
            ) : (
              <button className="account-form__add-btn" onClick={() => setPickingSite(true)}>
                {sitePicked ? ("isNew" in sitePicked ? `+ ${sitePicked.name} (new)` : sitePicked.name) : "Select site…"}
              </button>
            )
          ) : (
            <span className="account-form__site-readonly">{initial?.site_name}</span>
          )}

          <label>Username:</label>
          <input value={username} onChange={(e) => setUsername(e.target.value)} />

          <label>Display:</label>
          <input value={displayName} onChange={(e) => setDisplayName(e.target.value)} />

          <label>Alias:</label>
          <input value={alias} onChange={(e) => setAlias(e.target.value)} />

          <label>Notes:</label>
          <textarea value={notes} onChange={(e) => setNotes(e.target.value)} />

          {mode === "add" && (
            <>
              <label>Password:</label>
              <div className="modal__field-row">
                <input
                  type="text"
                  value={initialPassword}
                  onChange={(e) => setInitialPassword(e.target.value)}
                  placeholder="optional"
                />
                <button
                  type="button"
                  onClick={() => setGenOpen(true)}
                  disabled={busy}
                  aria-label="Generate password"
                  title="Generate password"
                  className="modal__gen-btn"
                >
                  {"\u{1F3B2}"}
                </button>
              </div>
            </>
          )}
        </div>
        {error && <div className="account-form__error">{error}</div>}
        <div className="modal__actions">
          <button onClick={onClose} disabled={busy}>Cancel</button>
          <button onClick={handleSave} disabled={!canSave || busy}>Save</button>
        </div>
        {genOpen && (
          <PasswordGeneratorPopover
            onChoose={(pw) => setInitialPassword(pw)}
            onClose={() => setGenOpen(false)}
          />
        )}
      </div>
    </div>
  );
}
