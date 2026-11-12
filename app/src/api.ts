// Types and calls shared with the Rust backend (src-tauri/src/model.rs).
import { invoke as tauriInvoke } from "@tauri-apps/api/core";

const inTauri = "__TAURI_INTERNALS__" in window;
const invoke = <T,>(cmd: string, args?: Record<string, unknown>): Promise<T> =>
  inTauri ? tauriInvoke<T>(cmd, args) : (import("./mock").then((m) => m.mockInvoke(cmd, args)) as Promise<T>);
export { inTauri };

export type PreviewItem = { kind: string; primary: string; detail: string; note?: string };
export type InfoRow = { name: string; detail: string };

export type Outcome =
  | { status: "plan"; id: number; title: string; total: number; items: PreviewItem[]; warnings: string[]; isUndo: boolean }
  | { status: "ambiguous"; name: string; candidates: string[]; folders: boolean }
  | { status: "info"; title: string; summary: string; rows: InfoRow[]; total: number; numbered?: boolean }
  | { status: "problem"; message: string }
  | { status: "not_understood"; text: string };

export type RunResult = { done: number; total: number; error: string | null; undoable: boolean; summary: string };

export type FolderAccess = { name: string; path: string; allowed: boolean };

/** A saved phrase ("run my usuals") that runs several commands at once. */
export type Shortcut = { phrase: string; commands: string[] };

/** The user's own name for a folder ("dev" for ~/Documents/Dev). */
export type Nickname = { name: string; path: string };

export type Settings = { hotkey: string; extraFolders: string[]; startAtLogin: boolean; onboarded: boolean; shortcuts: Shortcut[]; nicknames: Nickname[] };
/** "This was wrong" reports, saved only on this Mac. */
export type Reports = { count: number; path: string };
export type ResultScope = { count: number; visible: number; label: string };
export type CommandReply = { outcome: Outcome; scope: ResultScope | null };

export const api = {
  plan: (text: string, choices: Record<string, string>, requestId: number) => invoke<CommandReply>("plan_command", { text, choices, requestId }),
  clearResultScope: () => invoke<void>("clear_result_scope"),
  resultScope: () => invoke<ResultScope | null>("result_scope"),
  searchProgress: (requestId: number) => invoke<number>("search_progress", { requestId }),
  openHex: () => inTauri ? invoke<void>("open_hex") : Promise.resolve(window.open("https://anoma.ly/hex/", "_blank", "noopener,noreferrer")).then(() => {}),
  confirm: (id: number) => invoke<RunResult>("confirm_plan", { id }),
  cancel: () => invoke<void>("cancel_plan"),
  hide: (requestId: number) => invoke<void>("hide_window", { requestId }),
  reportWrong: (command: string, outcome: unknown, note: string) => invoke<Reports>("report_wrong", { command, outcome, note }),
  reportsSummary: () => invoke<Reports>("reports_summary"),
  clearReports: () => invoke<Reports>("clear_reports"),
  revealReports: () => invoke<void>("reveal_reports"),
  teachPhrase: (phrase: string, command: string) => invoke<Settings>("teach_phrase", { phrase, command }),
  rememberFolder: (name: string, path: string) => invoke<Settings>("remember_folder", { name, path }),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),
  allowedFolders: () => invoke<string[]>("allowed_folders"),
  requestFolderAccess: () => invoke<FolderAccess[]>("request_folder_access"),
  openPrivacySettings: () => invoke<void>("open_privacy_settings"),
  /** The Finder folder picker. Resolves to null if the user cancels. */
  pickFolder: async (): Promise<string | null> => {
    if (!inTauri) return "/Volumes/External";
    const { open } = await import("@tauri-apps/plugin-dialog");
    return open({ directory: true, title: "Add a folder" });
  },
};

/** "~/Downloads/a.pdf" from an absolute path, for display. */
export function tildify(p: string, home?: string): string {
  const h = home ?? p.match(/^\/Users\/[^/]+/)?.[0];
  return h && p.startsWith(h) ? "~" + p.slice(h.length) : p;
}
