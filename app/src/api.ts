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
  | { status: "ambiguous"; name: string; candidates: string[] }
  | { status: "info"; title: string; summary: string; rows: InfoRow[]; total: number }
  | { status: "problem"; message: string }
  | { status: "not_understood"; text: string };

export type RunResult = { done: number; total: number; error: string | null; undoable: boolean; summary: string };

/** A saved phrase ("run my usuals") that runs several commands at once. */
export type Shortcut = { phrase: string; commands: string[] };

export type Settings = { hotkey: string; extraFolders: string[]; startAtLogin: boolean; onboarded: boolean; shortcuts: Shortcut[] };

export const api = {
  plan: (text: string, choices: Record<string, string>) => invoke<Outcome>("plan_command", { text, choices }),
  confirm: (id: number) => invoke<RunResult>("confirm_plan", { id }),
  cancel: () => invoke<void>("cancel_plan"),
  hide: () => invoke<void>("hide_window"),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),
  allowedFolders: () => invoke<string[]>("allowed_folders"),
};

/** "~/Downloads/a.pdf" from an absolute path, for display. */
export function tildify(p: string, home?: string): string {
  const h = home ?? p.match(/^\/Users\/[^/]+/)?.[0];
  return h && p.startsWith(h) ? "~" + p.slice(h.length) : p;
}
