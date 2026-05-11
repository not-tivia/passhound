import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type {
  AccountSummary,
  AccountDetail,
  GuiError,
  Mapping,
  PreviewResult,
  CommitResult,
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
  listAccounts: (filter?: string) =>
    call<AccountSummary[]>("list_accounts", { filter: filter || null }),
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
};
