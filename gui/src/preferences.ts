// Browser-side preferences persisted in localStorage. Single JSON blob under
// one key for easy migration. Read-mode tolerates malformed/tampered storage
// by falling back to defaults; write-mode swallows quota errors silently.

const SCHEMA_VERSION = 1;
const STORAGE_KEY = "passhound.preferences.v1";

export type PrimaryIdentity = "username" | "display_name";
export type ColumnId = "site" | "user_display" | "last";
export type BaseWordsSortMode = "usage" | "alpha" | "last_seen";

interface Preferences {
  schema_version: number;
  vault_list_primary: PrimaryIdentity;
  vault_list_column_order: ColumnId[];
  baseWordsSortMode: BaseWordsSortMode;
}

const DEFAULT: Preferences = {
  schema_version: SCHEMA_VERSION,
  vault_list_primary: "username",
  vault_list_column_order: ["site", "user_display", "last"],
  baseWordsSortMode: "usage",
};

const ALL_COLUMN_IDS: ColumnId[] = ["site", "user_display", "last"];

function sanitizeBaseWordsSortMode(input: unknown): BaseWordsSortMode {
  return input === "usage" || input === "alpha" || input === "last_seen"
    ? (input as BaseWordsSortMode)
    : "usage";
}

function load(): Preferences {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT;
    const parsed = JSON.parse(raw) as Partial<Preferences>;
    return {
      schema_version: SCHEMA_VERSION,
      vault_list_primary:
        parsed.vault_list_primary === "display_name" ? "display_name" : "username",
      vault_list_column_order: sanitizeColumnOrder(parsed.vault_list_column_order),
      baseWordsSortMode: sanitizeBaseWordsSortMode(parsed.baseWordsSortMode),
    };
  } catch {
    return DEFAULT;
  }
}

function sanitizeColumnOrder(input: unknown): ColumnId[] {
  if (!Array.isArray(input)) return DEFAULT.vault_list_column_order;
  const known = new Set<ColumnId>(ALL_COLUMN_IDS);
  const filtered: ColumnId[] = [];
  for (const x of input) {
    if (typeof x === "string" && known.has(x as ColumnId)) {
      const id = x as ColumnId;
      if (!filtered.includes(id)) filtered.push(id);
    }
  }
  // Append any missing standard ids (e.g. after a future column addition).
  for (const id of ALL_COLUMN_IDS) {
    if (!filtered.includes(id)) filtered.push(id);
  }
  return filtered;
}

function save(prefs: Preferences): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(prefs));
  } catch {
    // Storage quota or disabled — defaults will apply next read.
  }
}

export function getVaultListPrimary(): PrimaryIdentity {
  return load().vault_list_primary;
}

export function setVaultListPrimary(value: PrimaryIdentity): void {
  const prefs = load();
  prefs.vault_list_primary = value;
  save(prefs);
}

export function getVaultListColumnOrder(): ColumnId[] {
  return load().vault_list_column_order;
}

export function setVaultListColumnOrder(order: ColumnId[]): void {
  const prefs = load();
  prefs.vault_list_column_order = sanitizeColumnOrder(order);
  save(prefs);
}

export function getBaseWordsSortMode(): BaseWordsSortMode {
  return load().baseWordsSortMode;
}

export function setBaseWordsSortMode(mode: BaseWordsSortMode): void {
  const prefs = load();
  prefs.baseWordsSortMode = sanitizeBaseWordsSortMode(mode);
  save(prefs);
}
