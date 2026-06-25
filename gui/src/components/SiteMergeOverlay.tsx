import { useEffect, useState } from "react";
import { api } from "../api";
import type { MergeGroupView, SiteSummary, GuiError } from "../types";

interface SiteMergeOverlayProps {
  onClose: () => void;
  onLockedError: () => void;
  onMutated: () => void;
}

export default function SiteMergeOverlay({ onClose, onLockedError, onMutated }: SiteMergeOverlayProps) {
  const [groups, setGroups] = useState<MergeGroupView[] | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<string | null>(null);

  // Manual merge state
  const [sites, setSites] = useState<SiteSummary[]>([]);
  const [survivorId, setSurvivorId] = useState<number | null>(null);
  const [loserId, setLoserId] = useState<number | null>(null);

  const handleErr = (e: unknown) => {
    if ((e as GuiError).kind === "Locked") onLockedError();
  };

  const load = () =>
    api.listSiteMergeGroups().then((gs) => {
      setGroups(gs);
      setSelected(new Set(gs.map((g) => g.canonical)));
    }, handleErr);

  useEffect(() => {
    load();
    api.listSites().then(setSites, handleErr);
  }, []);

  const toggle = (set: Set<string>, key: string, setter: (s: Set<string>) => void) => {
    const next = new Set(set);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    setter(next);
  };

  const selectedGroups = groups ? groups.filter((g) => selected.has(g.canonical)) : [];

  const doMerge = async () => {
    if (selectedGroups.length === 0) return;
    const totalRows = selectedGroups.reduce((n, g) => n + (g.members.length - 1), 0);
    const ok = confirm(
      `Merge ${selectedGroups.length} group${selectedGroups.length === 1 ? "" : "s"}? ` +
        `This removes ${totalRows} duplicate site row${totalRows === 1 ? "" : "s"} and moves their accounts ` +
        `under one entry each. Accounts and passwords are preserved.`,
    );
    if (!ok) return;
    setBusy(true);
    try {
      const res = await api.mergeSites(
        selectedGroups.map((g) => ({
          survivor_id: g.survivor_id,
          loser_ids: g.members.filter((m) => m.site_id !== g.survivor_id).map((m) => m.site_id),
        })),
      );
      setResult(
        `Merged ${res.groups_merged} group${res.groups_merged === 1 ? "" : "s"}, ` +
          `removed ${res.rows_removed} row${res.rows_removed === 1 ? "" : "s"}` +
          (res.skipped ? `, skipped ${res.skipped}` : "") +
          ".",
      );
      onMutated();
      await load();
    } catch (e) {
      handleErr(e);
    } finally {
      setBusy(false);
    }
  };

  const doManualMerge = async () => {
    if (survivorId === null || loserId === null || survivorId === loserId) return;
    const sv = sites.find((s) => s.id === survivorId);
    const ls = sites.find((s) => s.id === loserId);
    if (
      !confirm(
        `Merge "${ls?.name}" into "${sv?.name}"? Accounts move to "${sv?.name}"; "${ls?.name}" becomes an alias.`
      )
    )
      return;
    try {
      await api.mergeNamedSites(survivorId, [loserId]);
      setLoserId(null);
      onMutated();
      await load();
      api.listSites().then(setSites, handleErr);
    } catch (e) {
      handleErr(e);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal modal--site-merge" onClick={(e) => e.stopPropagation()}>
        <h2>Merge duplicate sites</h2>
        <p className="site-merge__intro">
          Sites that point to the same service are grouped below. Merging keeps one entry and moves all
          accounts under it. Nothing is deleted except the empty duplicate site rows.
        </p>
        <p className="site-merge__tip">Tip: back up your vault file before merging.</p>
        {result && <p className="site-merge__result">{result}</p>}

        {groups === null && <p className="modal__empty">{"\u2026 Loading"}</p>}
        {groups !== null && groups.length === 0 && <p className="modal__empty">No duplicate sites found.</p>}

        {groups !== null && groups.length > 0 && (
          <>
            <div className="site-merge__count">
              {groups.length} group{groups.length === 1 ? "" : "s"} found
            </div>
            <ul className="site-merge__list">
              {groups.map((g) => {
                const isOpen = expanded.has(g.canonical);
                return (
                  <li key={g.canonical} className="site-merge__group">
                    <div className="site-merge__row">
                      <input
                        type="checkbox"
                        checked={selected.has(g.canonical)}
                        onChange={() => toggle(selected, g.canonical, setSelected)}
                      />
                      <button
                        className="site-merge__chevron"
                        onClick={() => toggle(expanded, g.canonical, setExpanded)}
                        aria-label="Toggle member rows"
                      >
                        {isOpen ? "\u25BE" : "\u25B8"}
                      </button>
                      <span className="site-merge__brand">{g.clean_name}</span>
                      <span className="site-merge__meta">
                        {g.members.length} rows -&gt; 1 {"\u00B7"} {g.total_accounts} account
                        {g.total_accounts === 1 ? "" : "s"}
                      </span>
                    </div>
                    {isOpen && (
                      <ul className="site-merge__members">
                        {g.members.map((m) => (
                          <li
                            key={m.site_id}
                            className={
                              m.site_id === g.survivor_id
                                ? "site-merge__member site-merge__member--survivor"
                                : "site-merge__member"
                            }
                          >
                            <span className="site-merge__member-name">{m.name}</span>
                            {m.site_id === g.survivor_id && <span className="site-merge__keep">keep</span>}
                            <span className="site-merge__member-count">{m.account_count} acct</span>
                          </li>
                        ))}
                      </ul>
                    )}
                  </li>
                );
              })}
            </ul>
          </>
        )}

        <div className="site-merge__manual">
          <div className="site-merge__manual-label">Manual merge (different names)</div>
          <div className="site-merge__manual-row">
            <div>
              <label className="site-merge__manual-field-label">Merge into (survivor)</label>
              <select
                value={survivorId ?? ""}
                onChange={(e) => setSurvivorId(e.target.value ? Number(e.target.value) : null)}
              >
                <option value="">-- choose site --</option>
                {sites.map((s) => (
                  <option key={s.id} value={s.id}>{s.name}</option>
                ))}
              </select>
            </div>
            <div>
              <label className="site-merge__manual-field-label">Merge from (loser)</label>
              <select
                value={loserId ?? ""}
                onChange={(e) => setLoserId(e.target.value ? Number(e.target.value) : null)}
              >
                <option value="">-- choose site --</option>
                {sites.map((s) => (
                  <option key={s.id} value={s.id}>{s.name}</option>
                ))}
              </select>
            </div>
            <button
              className="modal__btn--primary"
              disabled={survivorId === null || loserId === null || survivorId === loserId}
              onClick={doManualMerge}
            >
              Merge
            </button>
          </div>
        </div>

        <div className="modal__actions">
          <button onClick={onClose}>Close</button>
          {groups !== null && groups.length > 0 && (
            <button
              className="modal__btn--primary"
              disabled={busy || selectedGroups.length === 0}
              onClick={doMerge}
            >
              {busy ? "Merging\u2026" : `Merge selected (${selectedGroups.length})`}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
