// Mirrors the Rust types in gui/src/commands.rs. Manual sync — small surface
// in 4.1; specta/ts-rs codegen is out of scope.

export interface AccountSummary {
  id: number;
  site_name: string;
  username: string | null;
  display_name: string | null;
  last_changed: string | null;
  category: string | null;
  tags: TagSummary[];
}

export interface AccountDetail {
  id: number;
  site_id: number;
  site_name: string;
  site_url: string | null;
  site_category: string | null;
  site_abbreviations: string[];
  site_notes: string | null;
  username: string | null;
  display_name: string | null;
  alias: string | null;
  notes: string | null;
  history: HistoryEntry[];
  tags: TagSummary[];
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
  | "EmptyVault"
  | "EraNotFound"
  | "NoActiveRecovery"
  | "RankOutOfBounds"
  | "NoPendingImport"
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
  display_name: number | null;
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
  display_name: string | null;
  password_length: number;
  notes: string | null;
}

export interface PreviewPartial {
  site: string | null;
  url: string | null;
  username: string | null;
  display_name: string | null;
  has_password: boolean;
  notes: string | null;
}

export interface PreviewDiagnostic {
  row: number;
  raw: string;
  reason: string;
  parsed: PreviewPartial;
}

export interface RowPatch {
  row: number;
  site: string | null;
  password: string | null;
}

export interface PreviewResult {
  headers: string[];
  detected_mapping: Mapping;
  effective_mapping: Mapping;
  counts: PreviewCounts;
  sample_rows: SampleRow[];
  diagnostics: PreviewDiagnostic[];
}

export interface CommitResult {
  import_id: number;
  counts: PreviewCounts;
}

// Phase 4.7 — Account mutation

export interface SiteSummary {
  id: number;
  name: string;
}

// Phase 4.6 — Tags

export interface Tag {
  id: number;
  name: string;
  created_at: string;
}

export interface TagWithCount {
  id: number;
  name: string;
  account_count: number;
}

export interface TagSummary {
  id: number;
  name: string;
}

// Phase 4.4 — Attachments

export interface AttachmentSummary {
  id: number;
  account_id: number;
  filename: string;
  mime_type: string;
  size_bytes: number;
  created_at: string;
}

export interface AttachmentRead {
  id: number;
  filename: string;
  mime_type: string;
  size_bytes: number;
  bytes_base64: string;
}

// Phase 4.8 — Recovery

export interface RecoverFilters {
  site: string | null;
  account: string | null;
  era: string | null;
  hint: string | null;
  limit: number;
  minLength: number | null;
  requireSymbol: boolean;
  requireDigit: boolean;
}

export interface RuleTag {
  tag: string;
  name: string;
}

export interface ScoreBreakdown {
  site: number;              site_weighted: number;
  hint: number;              hint_weighted: number;
  freq: number;              freq_weighted: number;
  fav: number;               fav_weighted: number;
  len: number;               len_weighted: number;
  orig_casing: number;       orig_casing_weighted: number;
  clean_pattern: number;     clean_pattern_weighted: number;
  history_seed: number;      history_seed_weighted: number;
  history_descendant: number; history_descendant_weighted: number;
  multiplier: number;
  total: number;
}

export interface CandidateView {
  rank: number;
  score: number;
  provenance: RuleTag[];
  breakdown: ScoreBreakdown | null;
}

export interface EraSummary {
  id: number;
  name: string;
  start_date: string | null;
  end_date: string | null;
  notes: string | null;
}

export interface EraFormArgs {
  name: string;
  start_date: string | null;  // "YYYY-MM-DD" or null
  end_date: string | null;
  notes: string | null;
}

// Phase 4.9 — Base Words

export interface BaseWordView {
  id: number;
  word: string;
  is_favorite: boolean;
  manual_override: boolean;
  usage_count: number;
  first_seen_at: string | null;
  last_seen_at: string | null;
}

export interface AnalyzeReportView {
  tokens_seen: number;
  base_words_written: number;
  favorites_set: number;
}

// Phase 4.10 — Settings

export interface SettingsView {
  idle_lock_seconds: number;
  clipboard_clear_seconds: number;
  analyze_top_n: number;
  default_reveal: boolean;
  reveal_clear_seconds: number;
}

export type SettingChange =
  | { key: "idle_lock_seconds"; value: number }
  | { key: "clipboard_clear_seconds"; value: number }
  | { key: "analyze_top_n"; value: number }
  | { key: "default_reveal"; value: boolean }
  | { key: "reveal_clear_seconds"; value: number };

// Phase 4.12 — Recovery feedback

export interface RecordFeedbackPayload {
  accountId: number | null;
  provenance: string[];
  score: number;
  rank: number;
  worked: boolean;
  length: number;
  hasDigit: boolean;
  hasSymbol: boolean;
  hasUpper: boolean;
  hasLower: boolean;
}

// Phase 4.14 — Small items bundle

export interface AddPasswordPayload {
  accountId: number;
  plaintext: string;
  source?: string;
}

export interface UpdateSitePayload {
  name: string;
  url: string | null;
  category: string | null;
  abbreviations: string[];
  notes: string | null;
}

export interface GeneratorOptionsPayload {
  length: number;
  lowercase: boolean;
  uppercase: boolean;
  digits: boolean;
  symbols: boolean;
  avoidAmbiguous: boolean;
}
