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
