// Checks the latest GitHub release for a newer Voxi, downloads it with
// progress, installs it, and restarts. Updates are signed; the app only
// installs one whose signature matches the public key in tauri.conf.json.

import { useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { api, errorMessage, inTauri } from "./api";

type UpdatePhase =
  | { k: "idle" }
  | { k: "checking" }
  | { k: "latest" }
  | { k: "downloading"; version: string; notes: string[]; done: number; total: number | null }
  | { k: "restarting"; version: string; notes: string[] }
  | { k: "relaunchFailed"; notes: string[] }
  | { k: "error"; message: string };

export type UpdateState = UpdatePhase & { commandsAdded: boolean };

let state: UpdateState = { k: "idle", commandsAdded: false };
let checkId = 0;
const listeners = new Set<() => void>();
const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
};
const snapshot = () => state;
function setState(next: UpdatePhase, commandsAdded = state.commandsAdded) {
  state = { ...next, commandsAdded };
  for (const listener of listeners) listener();
}

export function useUpdateState(): UpdateState {
  return useSyncExternalStore(subscribe, snapshot);
}

export function canCheckForUpdates(current: UpdateState): boolean {
  return current.k === "idle" || current.k === "latest" || current.k === "error";
}

/** The "What's new" bullet points from a release's notes. */
export function whatsNew(body: string | undefined): string[] {
  if (!body) return [];
  const section = body.split(/^## /m).find((s) => /^what's new/i.test(s)) ?? body;
  return section
    .split("\n")
    .filter((l) => l.startsWith("- "))
    .map((l) => l.slice(2).trim())
    .slice(0, 6);
}

// Without these, a stalled connection leaves the button loading forever.
const CHECK_TIMEOUT_MS = 20_000;
const DOWNLOAD_TIMEOUT_MS = 10 * 60_000;

export async function appVersion(): Promise<string> {
  return inTauri ? getVersion() : "0.1.0";
}

export async function checkAndInstall(): Promise<void> {
  if (!canCheckForUpdates(state)) return;
  const id = ++checkId;
  setState({ k: "checking" }, false);
  // A vocabulary result can arrive after the updater finishes or a new check starts.
  void api.refreshVocabulary().then((result) => {
    if (result === "updated" && id === checkId) setState(state, true);
  }).catch(() => {});
  if (!inTauri) return fakeUpdate();
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    const update = await check({ timeout: CHECK_TIMEOUT_MS });
    if (!update) return setState({ k: "latest" });
    const notes = whatsNew(update.body);
    let done = 0;
    let total: number | null = null;
    setState({ k: "downloading", version: update.version, notes, done, total });
    await update.downloadAndInstall((e) => {
      if (state.k !== "downloading") return;
      if (e.event === "Started") total = e.data.contentLength ?? null;
      if (e.event === "Progress") done += e.data.chunkLength;
      setState({ k: "downloading", version: update.version, notes, done, total });
    }, { timeout: DOWNLOAD_TIMEOUT_MS });
    setState({ k: "restarting", version: update.version, notes });
    try {
      const { relaunch } = await import("@tauri-apps/plugin-process");
      await relaunch();
    } catch {
      signalUpdateFailure();
      setState({ k: "relaunchFailed", notes });
    }
  } catch (e) {
    signalUpdateFailure();
    setState({ k: "error", message: `Couldn't update: ${errorMessage(e)}` });
  }
}

function signalUpdateFailure() {
  // Diagnostics accepts only this fixed signal.
  void invoke<void>("updater_failed").catch(() => {});
}

/** Browser preview only: walks through the states without a real download. */
async function fakeUpdate() {
  const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
  await wait(600);
  const total = 9_800_000;
  const notes = whatsNew("## What's new\n- Screenshots are found in every folder\n- Voxi remembers folder access between launches");
  for (let done = 0; done <= total; done += 700_000) {
    setState({ k: "downloading", version: "0.1.4", notes, done, total });
    await wait(120);
  }
  setState({ k: "restarting", version: "0.1.4", notes });
}
