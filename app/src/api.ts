// Types and calls shared with the Rust backend (src-tauri/src/model.rs).
import { invoke as tauriInvoke } from "@tauri-apps/api/core";

const inTauri = "__TAURI_INTERNALS__" in window;
const invoke = <T,>(cmd: string, args?: Record<string, unknown>): Promise<T> =>
  inTauri ? tauriInvoke<T>(cmd, args) : (import("./mock").then((m) => m.mockInvoke(cmd, args)) as Promise<T>);
export { inTauri };

/** Turn a Tauri command rejection (or browser-mock throw) into text that is
 *  safe to show in the UI. Backends sometimes reject with a bare string. */
/** A refused confirmation; `previewKept` means the preview is still confirmable. */
export type ConfirmError = { message: string; previewKept: boolean };
export function isConfirmError(e: unknown): e is ConfirmError {
  return typeof e === "object" && e !== null && typeof (e as ConfirmError).message === "string" && typeof (e as ConfirmError).previewKept === "boolean";
}

export function errorMessage(e: unknown): string {
  if (e instanceof Error && e.message.trim()) return e.message;
  if (typeof e === "string" && e.trim()) return e;
  if (e && typeof e === "object" && "message" in e && typeof e.message === "string" && e.message.trim()) return e.message;
  return "Something went wrong. Please try again.";
}

/** Failed settings saves can report the live hotkey after compensation. */
export function installedHotkeyAfterSave(e: unknown): string | undefined {
  if (e && typeof e === "object" && "installedHotkey" in e && typeof e.installedHotkey === "string") return e.installedHotkey;
}

/** `skippable` steps can be unticked to leave them out before confirming. */
export type PreviewItem = { kind: string; primary: string; detail: string; note?: string; skippable?: boolean };
/** A preview step loaded after the first page; `step` counts from 0. */
export type PreviewRow = PreviewItem & { step: number };
export type InfoRow = { name: string; detail: string };

export type FirstRunPolicy = "immediate" | "oneTimeDeferred" | "repeatLater" | "runNowAndRepeat";

export type Outcome =
  | { status: "plan"; id: number; title: string; total: number; items: PreviewItem[]; noted: number; scopeNotice?: string; warnings: string[]; modelInterpreted: boolean; isUndo: boolean; firstRun: FirstRunPolicy; scheduledFor?: string | null; repeats?: string | null }
  | { status: "ambiguous"; name: string; candidates: string[]; folders: boolean; reason: "multiple_matches" | "suggested_match" | "incomplete_search"; note?: string }
  | { status: "info"; title: string; summary: string; rows: InfoRow[]; total: number; numbered?: boolean }
  | { status: "problem"; message: string }
  | { status: "not_understood"; text: string };

export type SeriesState = "active" | "paused" | "stopped" | "completed" | "failed" | "firstRunPending" | "needsReview";
export type OccurrenceState = "waiting" | "running" | "finished" | "resultUnsaved";
export type ScheduledTask = { id: number; command: string; due: string; series: SeriesState; occurrence: OccurrenceState; summary: string; repeat?: string | null; choices?: Record<string, string> };

/** A needs-review scheduled task whose command is planned again for approval. */
export type Review = { id: number; command: string; choices: Record<string, string> };

export type RecoveryStatus =
  | { status: "clear" }
  | { status: "busy" }
  | { status: "needs_review"; command: string; acknowledged: number; inFlight: Record<string, unknown> | null; reason: string | null };

export type RunResult = {
  status: "completed" | "cancelled" | "failed" | "partially_failed" | "scheduled" | "needs_review";
  entryId: number | null; done: number; total: number; error: string | null; cancelled: boolean; undoable: boolean; summary: string };

export type FolderAccess = { name: string; path: string; allowed: boolean };

/** A saved phrase ("run my usuals") that runs several commands at once. */
export type Shortcut = { phrase: string; commands: string[] };

/** The user's own name for a folder ("dev" for ~/Documents/Dev). */
export type Nickname = { name: string; path: string };

/** `excludedFolders` sit inside allowed ones; the deepest matching folder wins. */
export type Settings = { hotkey: string; extraFolders: string[]; excludedFolders: string[]; startAtLogin: boolean; showMenuBarIcon: boolean; onboarded: boolean; ranFirstCommand: boolean; shortcuts: Shortcut[]; nicknames: Nickname[] };
/** "This was wrong" reports, saved only on this Mac. */
export type Reports = { count: number; path: string };
export type CommandRequestResult = { status: "sent" | "rate_limited" } | { status: "failed"; message: string };
export type ResultScope = { count: number; visible: number; label: string };
/** Counts and a time bucket from the request that produced the result. */
export type SearchStats = {
  visited: number;
  too_deep: number;
  unreadable: number;
  elapsed_ms_bucket: "under100" | "under1000" | "under5000" | "at_least5000";
};
/** A null generation leaves the view's results list alone. */
export type CommandReply = { outcome: Outcome; scope: ResultScope | null; generation: number | null; search_stats?: SearchStats };
/** What a newly opened command view starts from; reading it twice gives the same answer. */
export type Opening = { scope: ResultScope | null; generation: number; notice: string | null };

export const api = {
  refreshVocabulary: () => invoke<"updated" | "unchanged" | "failed">("refresh_vocabulary"),
  /** The command box changed: the menu bar face talks along. Fire and forget. */
  trayTyping: () => invoke<void>("tray_typing").catch(() => {}),
  recoveryStatus: () => invoke<RecoveryStatus>("recovery_status"),
  acknowledgeRecovery: () => invoke<void>("acknowledge_recovery"),
  schedulerNotice: () => invoke<string | null>("scheduler_notice"),
  scheduledTasks: () => invoke<ScheduledTask[]>("scheduled_tasks"),
  cancelScheduledTask: (id: number) => invoke<void>("cancel_scheduled_task", { id }),
  pauseScheduledTask: (id: number) => invoke<void>("pause_scheduled_task", { id }),
  resumeScheduledTask: (id: number) => invoke<void>("resume_scheduled_task", { id }),
  /** `replaces` is a needs-review task the planned repeating task takes the place of once confirmed. */
  plan: (text: string, choices: Record<string, string>, requestId: number, undoEntryId?: number, replaces?: number) =>
    invoke<CommandReply>("plan_command", { text, choices, requestId, undoEntryId, replaces: replaces ?? null }),
  /** Clears the results list, only if it is still the one at `generation` when given. */
  clearResultScope: (generation?: number | null) => invoke<void>("clear_result_scope", { generation: generation ?? null }),
  resultScope: () => invoke<ResultScope | null>("result_scope"),
  openingContext: (requestId: number) => invoke<Opening | null>("opening_context", { requestId }),
  searchProgress: (requestId: number) => invoke<number>("search_progress", { requestId }),
  openHex: () => inTauri ? invoke<void>("open_hex") : Promise.resolve(window.open("https://anoma.ly/hex/", "_blank", "noopener,noreferrer")).then(() => {}),
  /** Runs the pending plan, leaving out the 0-based steps in `skip`. */
  confirm: (id: number, skip: number[] = []) => invoke<RunResult>("confirm_plan", { id, skip }),
  /** More of the pending preview from `offset`, or only the steps with notes. */
  previewPage: (id: number, offset: number, onlyNotes: boolean) => invoke<PreviewRow[]>("preview_page", { id, offset, onlyNotes }),
  /** False when the run had already finished, so there was nothing to stop. */
  stopExecution: (id: number, scheduledFor?: string) => invoke<boolean>("cancel_execution", { id, scheduledFor }),
  cancel: () => invoke<void>("cancel_plan"),
  hide: (requestId: number) => invoke<void>("hide_window", { requestId }),
  reportWrong: (command: string, outcome: unknown, note: string, searchStats?: SearchStats) => invoke<Reports>("report_wrong", { command, outcome, note, searchStats }),
  sendCommandRequest: (command: string, note?: string) =>
    invoke<CommandRequestResult>("send_command_request", { command, note }),
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
