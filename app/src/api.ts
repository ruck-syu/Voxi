// Types and calls shared with the Rust backend (src-tauri/src/model.rs).
import { invoke as tauriInvoke } from "@tauri-apps/api/core";

const inTauri = "__TAURI_INTERNALS__" in window;
const invoke = <T,>(cmd: string, args?: Record<string, unknown>): Promise<T> =>
  inTauri ? tauriInvoke<T>(cmd, args) : (import("./mock").then((m) => m.mockInvoke(cmd, args)) as Promise<T>);
export { inTauri };

/** Turn a Tauri command rejection (or browser-mock throw) into text that is
 *  safe to show in the UI. Backends sometimes reject with a bare string. */
export function errorMessage(e: unknown): string {
  if (e instanceof Error && e.message.trim()) return e.message;
  if (typeof e === "string" && e.trim()) return e;
  return "Something went wrong. Please try again.";
}

/** `skippable` steps can be unticked to leave them out before confirming. */
export type PreviewItem = { kind: string; primary: string; detail: string; note?: string; skippable?: boolean };
/** A preview step loaded after the first page; `step` counts from 0. */
export type PreviewRow = PreviewItem & { step: number };
export type InfoRow = { name: string; detail: string };

export type Outcome =
  | { status: "plan"; id: number; title: string; total: number; items: PreviewItem[]; noted: number; warnings: string[]; isUndo: boolean; scheduledFor?: string | null; repeats?: string | null }
  | { status: "ambiguous"; name: string; candidates: string[]; folders: boolean }
  | { status: "info"; title: string; summary: string; rows: InfoRow[]; total: number; numbered?: boolean }
  | { status: "problem"; message: string }
  | { status: "not_understood"; text: string };

export type ScheduledTask = { id: number; command: string; due: string; status: "pending" | "running" | "paused" | "completed" | "failed" | "cancelled"; summary: string; repeat?: string | null };

export type RecoveryStatus = { command: string; acknowledged: number; inFlight: Record<string, unknown> | null };

export type RunResult = { done: number; total: number; error: string | null; cancelled: boolean; undoable: boolean; summary: string };

export type FolderAccess = { name: string; path: string; allowed: boolean };

/** A saved phrase ("run my usuals") that runs several commands at once. */
export type Shortcut = { phrase: string; commands: string[] };

/** The user's own name for a folder ("dev" for ~/Documents/Dev). */
export type Nickname = { name: string; path: string };

/** `excludedFolders` sit inside allowed ones; the deepest matching folder wins. */
export type Settings = { hotkey: string; extraFolders: string[]; excludedFolders: string[]; startAtLogin: boolean; onboarded: boolean; shortcuts: Shortcut[]; nicknames: Nickname[] };
/** "This was wrong" reports, saved only on this Mac. */
export type Reports = { count: number; path: string };
export type ResultScope = { count: number; visible: number; label: string };
export type CommandReply = { outcome: Outcome; scope: ResultScope | null };

export const api = {
  /** The command box changed: the menu bar face talks along. Fire and forget. */
  trayTyping: () => invoke<void>("tray_typing").catch(() => {}),
  recoveryStatus: () => invoke<RecoveryStatus | null>("recovery_status"),
  acknowledgeRecovery: () => invoke<void>("acknowledge_recovery"),
  schedulerNotice: () => invoke<string | null>("scheduler_notice"),
  scheduledTasks: () => invoke<ScheduledTask[]>("scheduled_tasks"),
  cancelScheduledTask: (id: number) => invoke<void>("cancel_scheduled_task", { id }),
  pauseScheduledTask: (id: number) => invoke<void>("pause_scheduled_task", { id }),
  resumeScheduledTask: (id: number) => invoke<void>("resume_scheduled_task", { id }),
  plan: (text: string, choices: Record<string, string>, requestId: number) => invoke<CommandReply>("plan_command", { text, choices, requestId }),
  clearResultScope: () => invoke<void>("clear_result_scope"),
  resultScope: () => invoke<ResultScope | null>("result_scope"),
  /** What Voxi left out of the last Finder selection, once. */
  selectionNotice: () => invoke<string | null>("selection_notice"),
  searchProgress: (requestId: number) => invoke<number>("search_progress", { requestId }),
  openHex: () => inTauri ? invoke<void>("open_hex") : Promise.resolve(window.open("https://anoma.ly/hex/", "_blank", "noopener,noreferrer")).then(() => {}),
  /** Runs the pending plan, leaving out the 0-based steps in `skip`. */
  confirm: (id: number, skip: number[] = []) => invoke<RunResult>("confirm_plan", { id, skip }),
  /** More of the pending preview from `offset`, or only the steps with notes. */
  previewPage: (id: number, offset: number, onlyNotes: boolean) => invoke<PreviewRow[]>("preview_page", { id, offset, onlyNotes }),
  stopExecution: (id: number, scheduledFor?: string) => invoke<void>("cancel_execution", { id, scheduledFor }),
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
  pickFolder: async (options: { title?: string; defaultPath?: string } = {}): Promise<string | null> => {
    if (!inTauri) return options.defaultPath ? `${options.defaultPath}/Archive` : "/Volumes/External";
    const { open } = await import("@tauri-apps/plugin-dialog");
    return open({ directory: true, title: options.title ?? "Add a folder", defaultPath: options.defaultPath });
  },
  /** The user's home folder, to turn "~/Documents" into a real path. */
  homeDir: async (): Promise<string> => {
    if (!inTauri) return "/Users/you";
    const { homeDir } = await import("@tauri-apps/api/path");
    return (await homeDir()).replace(/\/$/, "");
  },
};

/** "~/Downloads/a.pdf" from an absolute path, for display. */
export function tildify(p: string, home?: string): string {
  const h = home ?? p.match(/^\/Users\/[^/]+/)?.[0];
  return h && p.startsWith(h) ? "~" + p.slice(h.length) : p;
}
