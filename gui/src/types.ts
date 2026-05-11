// Mirrors the Rust types in gui/src/commands.rs. Manual sync — small surface
// in 4.1; specta/ts-rs codegen is out of scope.

export interface AccountSummary {
  id: number;
  site_name: string;
  username: string | null;
  last_changed: string | null;
  category: string | null;
}

export interface AccountDetail {
  id: number;
  site_name: string;
  site_url: string | null;
  site_category: string | null;
  site_abbreviations: string[];
  username: string | null;
  history: HistoryEntry[];
}

export interface HistoryEntry {
  id: number;
  created_at: string;
  source: string;
  is_current: boolean;
  notes: string | null;
}

export type GuiErrorKind =
  | "NotFound"
  | "Locked"
  | "WrongPassword"
  | "AlreadyExists"
  | "InvalidInput"
  | "Internal";

export interface GuiError {
  kind: GuiErrorKind;
  message?: string;
}

// Phase 4.2 — CSV import

export interface Mapping {
  site: number | null;
  url: number | null;
  username: number | null;
  password: number;
  notes: number | null;
  created_at: number | null;
  extras_into_notes: number[];
}

export interface PreviewCounts {
  new: number;
  duplicates: number;
  merges: number;
  errors: number;
}

export interface SampleRow {
  site: string;
  username: string | null;
  password_length: number;
  notes: string | null;
}

export interface PreviewResult {
  headers: string[];
  detected_mapping: Mapping;
  effective_mapping: Mapping;
  counts: PreviewCounts;
  sample_rows: SampleRow[];
  diagnostics: string[];
}

export interface CommitResult {
  import_id: number;
  counts: PreviewCounts;
}
