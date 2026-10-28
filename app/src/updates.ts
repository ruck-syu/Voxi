// Checks the latest GitHub release for a newer Voxi, downloads it with
// progress, installs it, and restarts. Updates are signed; the app only
// installs one whose signature matches the public key in tauri.conf.json.

import { getVersion } from "@tauri-apps/api/app";
import { inTauri } from "./api";

export type UpdateState =
  | { k: "idle" }
  | { k: "checking" }
  | { k: "latest" }
  | { k: "downloading"; version: string; notes: string[]; done: number; total: number | null }
  | { k: "restarting"; version: string; notes: string[] }
  | { k: "error"; message: string };

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

export async function appVersion(): Promise<string> {
  return inTauri ? getVersion() : "0.1.0";
}

export async function checkAndInstall(onState: (s: UpdateState) => void): Promise<void> {
  if (!inTauri) return fakeUpdate(onState);
  const { check } = await import("@tauri-apps/plugin-updater");
  const { relaunch } = await import("@tauri-apps/plugin-process");
  onState({ k: "checking" });
  try {
    const update = await check();
    if (!update) return onState({ k: "latest" });
    const notes = whatsNew(update.body);
    let done = 0;
    let total: number | null = null;
    onState({ k: "downloading", version: update.version, notes, done, total });
    await update.downloadAndInstall((e) => {
      if (e.event === "Started") total = e.data.contentLength ?? null;
      if (e.event === "Progress") done += e.data.chunkLength;
      onState({ k: "downloading", version: update.version, notes, done, total });
    });
    onState({ k: "restarting", version: update.version, notes });
    await relaunch();
  } catch (e) {
    onState({ k: "error", message: `Couldn't update: ${e}` });
  }
}

/** Browser preview only: walks through the states without a real download. */
async function fakeUpdate(onState: (s: UpdateState) => void) {
  const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
  onState({ k: "checking" });
  await wait(600);
  const total = 9_800_000;
  const notes = whatsNew("## What's new\n- Screenshots are found in every folder\n- Voxi remembers folder access between launches");
  for (let done = 0; done <= total; done += 700_000) {
    onState({ k: "downloading", version: "0.1.4", notes, done, total });
    await wait(120);
  }
  onState({ k: "restarting", version: "0.1.4", notes });
}
