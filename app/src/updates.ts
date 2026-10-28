// Checks the latest GitHub release for a newer Voxi, downloads it with
// progress, installs it, and restarts. Updates are signed; the app only
// installs one whose signature matches the public key in tauri.conf.json.

import { getVersion } from "@tauri-apps/api/app";
import { inTauri } from "./api";

export type UpdateState =
  | { k: "idle" }
  | { k: "checking" }
  | { k: "latest" }
  | { k: "downloading"; version: string; done: number; total: number | null }
  | { k: "restarting"; version: string }
  | { k: "error"; message: string };

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
    let done = 0;
    let total: number | null = null;
    onState({ k: "downloading", version: update.version, done, total });
    await update.downloadAndInstall((e) => {
      if (e.event === "Started") total = e.data.contentLength ?? null;
      if (e.event === "Progress") done += e.data.chunkLength;
      onState({ k: "downloading", version: update.version, done, total });
    });
    onState({ k: "restarting", version: update.version });
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
  for (let done = 0; done <= total; done += 700_000) {
    onState({ k: "downloading", version: "0.1.4", done, total });
    await wait(120);
  }
  onState({ k: "restarting", version: "0.1.4" });
}
