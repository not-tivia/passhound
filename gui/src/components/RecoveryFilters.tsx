import { useEffect, useState } from "react";
import { api } from "../api";
import type {
  RecoverFilters,
  SiteSummary,
  AccountSummary,
  EraSummary,
  GuiError,
} from "../types";

interface RecoveryFiltersProps {
  filters: RecoverFilters;
  onChange: (next: RecoverFilters) => void;
  onRun: () => void;
  running: boolean;
  onLockedError: () => void;
}

export default function RecoveryFilters({
  filters,
  onChange,
  onRun,
  running,
  onLockedError,
}: RecoveryFiltersProps) {
  const [sites, setSites] = useState<SiteSummary[]>([]);
  const [accounts, setAccounts] = useState<AccountSummary[]>([]);
  const [eras, setEras] = useState<EraSummary[]>([]);

  useEffect(() => {
    api.listSites().then(setSites, (e) => {
      if ((e as GuiError).kind === "Locked") onLockedError();
    });
    api.listEras().then(setEras, (e) => {
      if ((e as GuiError).kind === "Locked") onLockedError();
    });
    api.listAccounts().then(setAccounts, (e) => {
      if ((e as GuiError).kind === "Locked") onLockedError();
    });
  }, [onLockedError]);

  const accountsForSite = filters.site
    ? accounts.filter((a) => a.site_name === filters.site)
    : accounts;

  const patch = (changes: Partial<RecoverFilters>) =>
    onChange({ ...filters, ...changes });

  return (
    <form
      className="recovery-filters"
      onSubmit={(e) => {
        e.preventDefault();
        onRun();
      }}
    >
      <label className="recovery-filters__label">SITE</label>
      <select
        className="recovery-filters__input"
        value={filters.site ?? ""}
        onChange={(e) =>
          patch({
            site: e.target.value || null,
            account: null,
          })
        }
      >
        <option value="">any</option>
        {sites.map((s) => (
          <option key={s.id} value={s.name}>
            {s.name}
          </option>
        ))}
      </select>

      <label className="recovery-filters__label">ACCOUNT</label>
      <select
        className="recovery-filters__input"
        value={filters.account ?? ""}
        onChange={(e) => patch({ account: e.target.value || null })}
      >
        <option value="">any</option>
        {accountsForSite.map((a) => (
          <option key={a.id} value={a.username ?? a.display_name ?? `#${a.id}`}>
            {a.username ?? a.display_name ?? `#${a.id}`}
          </option>
        ))}
      </select>

      <label className="recovery-filters__label">ERA</label>
      <select
        className="recovery-filters__input"
        value={filters.era ?? ""}
        onChange={(e) => patch({ era: e.target.value || null })}
      >
        <option value="">any</option>
        {eras.map((e) => (
          <option key={e.id} value={e.name}>
            {e.name}
          </option>
        ))}
      </select>

      <label className="recovery-filters__label">HINT</label>
      <input
        className="recovery-filters__input"
        type="text"
        value={filters.hint ?? ""}
        onChange={(e) => patch({ hint: e.target.value || null })}
        placeholder="moon"
      />

      <label className="recovery-filters__label">LIMIT</label>
      <input
        className="recovery-filters__input"
        type="number"
        min={1}
        max={5000}
        value={filters.limit}
        onChange={(e) => {
          const v = parseInt(e.target.value, 10);
          patch({ limit: Number.isFinite(v) && v > 0 ? v : 1 });
        }}
      />

      <label className="recovery-filters__label">MIN LENGTH</label>
      <input
        className="recovery-filters__input"
        type="number"
        min={1}
        value={filters.minLength ?? ""}
        onChange={(e) => {
          const v = parseInt(e.target.value, 10);
          patch({ minLength: Number.isFinite(v) && v > 0 ? v : null });
        }}
      />

      <label className="recovery-filters__checkbox">
        <input
          type="checkbox"
          checked={filters.requireSymbol}
          onChange={(e) => patch({ requireSymbol: e.target.checked })}
        />
        Require symbol
      </label>
      <label className="recovery-filters__checkbox">
        <input
          type="checkbox"
          checked={filters.requireDigit}
          onChange={(e) => patch({ requireDigit: e.target.checked })}
        />
        Require digit
      </label>

      <button
        type="submit"
        className="recovery-filters__run"
        disabled={running}
      >
        {running ? "Running…" : "▶ Run recovery"}
      </button>
    </form>
  );
}
