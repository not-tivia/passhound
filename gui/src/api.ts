import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { readText } from "@tauri-apps/plugin-clipboard-manager";
import type {
  AccountSummary,
  AccountDetail,
  AttachmentRead,
  AttachmentSummary,
  GuiError,
  Mapping,
  PreviewResult,
  CommitResult,
  SiteSummary,
  TagSummary,
  TagWithCount,
  RecoverFilters,
  CandidateView,
  EraSummary,
  BaseWordView,
  AnalyzeReportView,
  SettingsView,
  SettingChange,
  RecordFeedbackPayload,
  UpdateSitePayload,
  GeneratorOptionsPayload,
} from "./types";

// Wrap Tauri's `invoke` so caller gets typed promises and a stable
// rejection shape (always GuiError, never raw string).
async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e: unknown) {
    if (typeof e === "object" && e !== null && "kind" in e) {
      throw e as GuiError;
    }
    throw { kind: "Internal", message: String(e) } as GuiError;
  }
}

export const api = {
  vaultExists: () => call<boolean>("vault_exists"),
  vaultCreate: (masterPw: string) => call<void>("vault_create", { masterPw }),
  vaultUnlock: (masterPw: string) => call<void>("vault_unlock", { masterPw }),
  vaultLock: () => call<void>("vault_lock"),
  listAccounts: (filter?: string, tagIds?: number[]) =>
    call<AccountSummary[]>("list_accounts", { filter: filter || null, tagIds: tagIds ?? null }),
  getAccount: (id: number) => call<AccountDetail>("get_account", { id }),
  revealPassword: (historyId: number) =>
    call<string>("reveal_password", { historyId }),
  copyToClipboard: (text: string) => call<void>("copy_to_clipboard", { text }),

  // Phase 4.2 — CSV import
  importCsvDryRun: (
    path: string,
    siteOverride: string | null,
    mapping: Mapping | null,
  ) =>
    call<PreviewResult>("import_csv_dry_run", {
      path,
      siteOverride,
      mapping,
    }),
  importCsvCommit: (
    path: string,
    siteOverride: string | null,
    mapping: Mapping | null,
  ) =>
    call<CommitResult>("import_csv_commit", {
      path,
      siteOverride,
      mapping,
    }),

  // Native file picker via @tauri-apps/plugin-dialog. Returns null if user
  // cancels.
  pickCsvFile: async (): Promise<string | null> => {
    const result = await openDialog({
      multiple: false,
      directory: false,
      filters: [{ name: "CSV", extensions: ["csv", "CSV"] }],
    });
    if (typeof result === "string") return result;
    return null;
  },

  // Phase 4.4 — Attachments
  listAttachments: (accountId: number) =>
    call<AttachmentSummary[]>("list_attachments", { accountId }),
  attachFile: (
    accountId: number,
    filename: string,
    mimeType: string,
    bytesBase64: string,
  ) =>
    call<AttachmentSummary>("attach_file", {
      accountId,
      filename,
      mimeType,
      bytesBase64,
    }),
  readAttachment: (attachmentId: number) =>
    call<AttachmentRead>("read_attachment", { attachmentId }),
  deleteAttachment: (attachmentId: number) =>
    call<void>("delete_attachment", { attachmentId }),
  deleteAccount: (id: number) => call<void>("delete_account", { accountId: id }),
  deletePassword: (historyId: number) => call<void>("delete_password", { historyId }),

  // Phase 4.6 — Tags
  listTags: () => call<TagWithCount[]>("list_tags"),
  createTag: (name: string) => call<TagSummary>("create_tag", { name }),
  renameTag: (tagId: number, newName: string) => call<void>("rename_tag", { tagId, newName }),
  deleteTag: (tagId: number) => call<void>("delete_tag", { tagId }),
  listAccountTags: (accountId: number) => call<TagSummary[]>("list_account_tags", { accountId }),
  assignTag: (accountId: number, tagId: number) => call<void>("assign_tag", { accountId, tagId }),
  unassignTag: (accountId: number, tagId: number) => call<void>("unassign_tag", { accountId, tagId }),
  bulkAssignTag: (accountIds: number[], tagId: number) => call<number>("bulk_assign_tag", { accountIds, tagId }),
  bulkUnassignTag: (accountIds: number[], tagId: number) => call<number>("bulk_unassign_tag", { accountIds, tagId }),
  bulkDeleteAccounts: (accountIds: number[]) => call<number>("bulk_delete_accounts", { accountIds }),

  // Phase 4.7 — Account mutation
  listSites: () => call<SiteSummary[]>("list_sites"),
  findOrCreateSite: (name: string) => call<SiteSummary>("find_or_create_site", { name }),
  addAccount: (fields: {
    siteId: number;
    username?: string | null;
    displayName?: string | null;
    alias?: string | null;
    notes?: string | null;
    initialPassword?: string | null;
  }) => call<number>("add_account", { fields: {
    site_id: fields.siteId,
    username: fields.username ?? null,
    display_name: fields.displayName ?? null,
    alias: fields.alias ?? null,
    notes: fields.notes ?? null,
    initial_password: fields.initialPassword ?? null,
  } }),
  updateAccount: (accountId: number, fields: {
    username?: string | null;
    displayName?: string | null;
    alias?: string | null;
    notes?: string | null;
  }) => call<void>("update_account", { accountId, fields: {
    username: fields.username ?? null,
    display_name: fields.displayName ?? null,
    alias: fields.alias ?? null,
    notes: fields.notes ?? null,
  } }),
  addPassword: (accountId: number, plaintext: string, source?: string) =>
    call<number>("add_password", { payload: { accountId, plaintext, source: source ?? null } }),
  promotePassword: (historyId: number) => call<void>("promote_password", { historyId }),

  // Native save dialog via @tauri-apps/plugin-dialog. Returns null if user cancels.
  saveFileDialog: async (defaultName: string): Promise<string | null> => {
    const result = await saveDialog({ defaultPath: defaultName });
    if (typeof result === "string") return result;
    return null;
  },

  // Phase 4.8 — Recovery
  recoverCandidates: (filters: RecoverFilters) =>
    call<CandidateView[]>("recover_candidates", { filters }),
  listEras: () => call<EraSummary[]>("list_eras"),

  // Phase 4.9 — Base Words
  listBaseWords: () => call<BaseWordView[]>("list_base_words"),
  promoteBaseWord: (id: number) => call<void>("promote_base_word", { id }),
  demoteBaseWord: (id: number) => call<void>("demote_base_word", { id }),
  analyzeBaseWords: () => call<AnalyzeReportView>("analyze_base_words"),

  // Phase 4.10 — Settings
  getSettings: () => call<SettingsView>("get_settings"),
  setSetting: (change: SettingChange) => call<void>("set_setting", { change }),
  copyToClipboardWithAutoClear: async (text: string, clipboardClearSeconds: number) => {
    await call<void>("copy_to_clipboard", { text });
    if (clipboardClearSeconds > 0) {
      window.setTimeout(async () => {
        try {
          // Read-before-clear: only wipe the clipboard if it still contains the
          // value PassHound wrote. If the user has since copied something else,
          // leave their clipboard alone — clearing it would be confusing and
          // the sensitive value is already gone.
          const current = await readText();
          if (current === text) {
            await call<void>("copy_to_clipboard", { text: "" });
          }
        } catch {
          // Clipboard read failure (e.g. permission denied on some platforms):
          // give up silently rather than unconditionally clearing.
        }
      }, clipboardClearSeconds * 1000);
    }
  },

  // Phase 4.11 — Master password change
  changeMasterPassword: (currentPw: string, newPw: string) =>
    call<void>("change_master_password", { currentPw, newPw }),

  // Phase 4.12 — Recovery feedback
  recordRecoveryFeedback: (payload: RecordFeedbackPayload) =>
    call<void>("record_recovery_feedback", { payload }),
  clearRecoveryFeedback: () => call<number>("clear_recovery_feedback"),

  // Phase 4.14 — Small items
  updateSite: (siteId: number, payload: UpdateSitePayload) =>
    call<void>("update_site", { siteId, payload }),
  generatePassword: (payload: GeneratorOptionsPayload) =>
    call<string>("generate_password", { payload }),
  addBaseWord: (text: string) =>
    call<BaseWordView>("add_base_word", { text }),

  // Phase 4.16 — Lazy reveal/copy via IPC
  revealCandidate: (rank: number) =>
    call<string>("reveal_candidate", { rank }),
  copyCandidate: (rank: number) =>
    call<void>("copy_candidate", { rank }),
};
