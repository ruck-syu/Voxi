// Browser-only stand-in for the Rust backend, so the window can be designed
// and reviewed with `npm run dev` outside Tauri. Never loaded inside the app.
import type { Outcome, RunResult, Settings, ScheduledTask } from "./api";

let settings: Settings = {
  hotkey: "Control+Alt+Space", extraFolders: [], excludedFolders: [], startAtLogin: false,
  // ?welcome previews the first-run page.
  onboarded: !new URLSearchParams(location.search).has("welcome"),
  shortcuts: [{ phrase: "run my usuals", commands: ["move screenshots from today to Screenshots", "delete installers from Downloads"] }],
  nicknames: [{ name: "dev", path: "~/Documents/Dev" }],
};
let reports = 0;
let scheduled: ScheduledTask[] = [];
/** The plan a preview page may be read from, as in the app; null once it is run, cancelled or replaced. */
let activePlan: number | null = null;
let pendingSchedule: (ScheduledTask & { runNow?: boolean; total?: number; intervalMs?: number }) | null = null;

// Browser-only permission scenarios, selected with ?mock=<name>:
//   permission-reject  request_folder_access rejects -> error state + Retry
//   permission-denied  request_folder_access resolves with every folder denied
//   permission-flaky   first request rejects, the next succeeds -> Retry works
//   settings-fail      open_privacy_settings rejects -> error + manual steps
// These only affect the dev mock; the Tauri app never loads this file.
const mockScenario = new URLSearchParams(location.search).get("mock");
let permissionAttempts = 0;
let recoveryReviewed = false;

// Mirrors parse::MAX_COMMAND_BYTES and settings::check_shortcut_commands so
// browser previews refuse exactly what the Rust backend refuses.
const MAX_COMMAND_BYTES = 16_384;
const TOO_LONG = "That command is too long.";
const utf8Bytes = (text: string): number => new TextEncoder().encode(text).length;

function plan(text: string, choices: Record<string, string> = {}): Outcome {
  const t = text.toLowerCase();
  if (t.includes("usuals"))
    return {
      status: "plan", id: 4, title: "Run my usuals: 2 commands", total: 3, noted: 0, isUndo: false,
      warnings: ["“delete installers from Downloads”: No files match in Downloads."],
      items: [
        { kind: "create_folder", primary: "Screenshots", detail: "new folder in ~/Desktop" },
        { kind: "move", primary: "Screenshot 2026-10-28 at 09.14.02.png", detail: "→ ~/Desktop/Screenshots" },
        { kind: "move", primary: "Screenshot 2026-10-28 at 11.40.19.png", detail: "→ ~/Desktop/Screenshots" },
      ],
    };
  if (t.includes("pdf"))
    return {
      status: "plan", id: 1, title: "Move 26 files to Documents", total: 26, noted: 2, isUndo: false,
      warnings: ["2 already in Documents, left alone."],
      items: [
        { kind: "move", primary: "Q3 board deck.pdf", detail: "→ ~/Documents", skippable: true },
        { kind: "move", primary: "lease-renewal-2026.pdf", detail: "→ ~/Documents", skippable: true },
        { kind: "move", primary: "report.pdf", detail: "→ ~/Documents/report 2.pdf", note: "“report.pdf” already exists, so this becomes “report 2.pdf”", skippable: true },
        { kind: "move", primary: "tax-receipt-march.pdf", detail: "→ ~/Documents", skippable: true },
        { kind: "move", primary: "W-2 2025.pdf", detail: "→ ~/Documents", skippable: true },
      ],
    };
  if (t.includes("clean"))
    return {
      status: "plan", id: 2, title: "Move 3 files from Downloads to the Trash", total: 3, noted: 0, isUndo: false, warnings: [],
      items: [
        { kind: "trash", primary: "Zoom.pkg", detail: "→ Trash" },
        { kind: "trash", primary: "Figma-124.3.dmg", detail: "→ Trash" },
        { kind: "trash", primary: "old-export.csv", detail: "→ Trash" },
      ],
    };
  if (t.includes("claude") && choices.claude)
    return {
      status: "plan", id: 3, title: `Move ${choices.claude.split("/").pop()} from Downloads to the Trash`, total: 1, noted: 0, isUndo: false, warnings: [],
      items: [{ kind: "trash", primary: choices.claude.split("/").pop()!, detail: "→ Trash" }],
    };
  if (t.includes("claude"))
    return {
      status: "ambiguous", name: "claude", folders: false,
      candidates: ["/Users/me/Downloads/Claude.dmg", "/Users/me/Downloads/Claude Setup.dmg", "/Users/me/Documents/claude-notes.txt"],
    };
  if (t.includes("size"))
    return {
      status: "info", title: "Downloads uses 18 GB", summary: "3,201 files in total. Largest items:", total: 40,
      rows: [
        { name: "Xcode_17.xip", detail: "11 GB" },
        { name: "Footage", detail: "4.2 GB · folder" },
        { name: "ubuntu-26.04.iso", detail: "2.1 GB" },
      ],
    };
  if (t.includes("missing")) return { status: "problem", message: "Couldn't find “budget.xlsx” in Desktop, Downloads, Documents, or Pictures." };
  return { status: "not_understood", text };
}

export async function mockInvoke(cmd: string, args: Record<string, unknown> = {}): Promise<unknown> {
  await new Promise((r) => setTimeout(r, 250));
  switch (cmd) {
    case "updater_failed": return;
    case "cancel_plan": pendingSchedule = null; activePlan = null; return;
    case "cancel_execution": return;
    case "search_progress": return 0;
    case "result_scope": return null;
    case "opening_context": return { scope: null, generation: 0, notice: null };
    case "clear_result_scope": return;
    case "recovery_status": return !recoveryReviewed && mockScenario?.startsWith("recovery") ? {
      command: "move PDFs to Documents", acknowledged: 2,
      inFlight: { kind: "move", from: "/Users/me/Downloads/a.pdf", to: "/Users/me/Documents/a.pdf" },
    } : null;
    case "acknowledge_recovery":
      if (mockScenario === "recovery-fail") throw new Error("Cannot archive recovery evidence. Try Mark as reviewed again.");
      recoveryReviewed = true; return;
    case "scheduler_notice": return null;
    case "scheduled_tasks": return scheduled;
    case "cancel_scheduled_task": {
      scheduled = scheduled.map((task) => task.id === args.id ? { ...task, status: "cancelled", summary: task.repeat ? "Stopped." : "Cancelled before running." } : task);
      return;
    }
    case "pause_scheduled_task":
    case "resume_scheduled_task": {
      const status = cmd === "pause_scheduled_task" ? "paused" : "pending";
      scheduled = scheduled.map((task) => task.id === args.id && task.repeat ? { ...task, status, summary: task.status === "paused" ? task.summary : "Made active again." } : task);
      return;
    }
    case "plan_command": {
      activePlan = null;
      const text = String(args.text);
      if (utf8Bytes(text) > MAX_COMMAND_BYTES) return { outcome: { status: "problem", message: TOO_LONG }, scope: null };
      pendingSchedule = null;
      const repeat = text.match(/\s+(?:every\s+(?:(\d+)\s+)?(days?|hours?)|daily|everyday|hourly)(?:\s+at\s+(\d{1,2})(?::(\d{2}))?\s*(am|pm)?)?$/i);
      if (repeat) {
        const outcome = plan(text.slice(0, repeat.index), args.choices as Record<string, string>);
        if (outcome.status !== "plan") return { outcome, scope: null };
        const n = Number(repeat[1] || 1);
        const unit = repeat[2] || (/hourly/i.test(repeat[0]) ? "hour" : "day");
        const intervalMs = n * (unit.startsWith("day") ? 86400000 : 3600000);
        const runNow = !repeat[3];
        const every = `every ${n === 1 ? "" : `${n} `}${unit}${runNow ? "" : ` at ${repeat[3]}:${repeat[4] || "00"} ${repeat[5] || ""}`}`;
        const due = new Date(Date.now() + intervalMs);
        if (!runNow) {
          due.setTime(Date.now());
          due.setHours(Number(repeat[3]) % (repeat[5] ? 12 : 24) + (repeat[5]?.toLowerCase() === "pm" ? 12 : 0), Number(repeat[4] || 0), 0, 0);
          if (due <= new Date()) due.setDate(due.getDate() + 1);
        }
        pendingSchedule = { id: scheduled.length + 1, command: text, due: due.toISOString(), status: "pending", summary: "", repeat: every, runNow, total: outcome.total, intervalMs };
        activePlan = outcome.id;
        return { outcome: { ...outcome, scheduledFor: due.toISOString(), repeats: runNow ? `Runs now, then ${every}` : every, warnings: [`${runNow ? `Runs now, then ${every}. Next run` : `Repeats ${every}, first on`} ${due.toLocaleString()}. Each run changes whatever matches at that time, without a preview, and can be undone from History.`, ...outcome.warnings] }, scope: null };
      }
      const timing = text.match(/\s+(?:at\s+(.+)|(after|in)\s+(\d+)\s+(hours?|minutes?))$/i);
      let outcome = plan(timing ? text.slice(0, timing.index) : text, args.choices as Record<string, string>);
      pendingSchedule = null;
      if (timing) {
        const due = new Date();
        if (timing[1]) {
          const clock = timing[1].match(/^(\d{1,2}):(\d{2})\s*(am|pm)?$/i);
          if (!clock) return { outcome: { status: "problem", message: "Use a time such as at 9:40 pm." }, scope: null };
          due.setHours(Number(clock[1]) % (clock[3] ? 12 : 24) + (clock[3]?.toLowerCase() === "pm" ? 12 : 0), Number(clock[2]), 0, 0);
          if (due <= new Date()) due.setDate(due.getDate() + 1);
        } else due.setTime(due.getTime() + Number(timing[3]) * (timing[4].startsWith("hour") ? 3600000 : 60000));
        if (text.toLowerCase().includes("screenshot")) outcome = plan("run my usuals");
        if (outcome.status === "plan") {
          outcome = { ...outcome, scheduledFor: due.toISOString(), warnings: [...outcome.warnings, `Scheduled for ${due.toLocaleString()}. Only the files shown here will be changed. Voxi must be running.`] };
          pendingSchedule = { id: scheduled.length + 1, command: text, due: due.toISOString(), status: "pending", summary: "" };
        }
      }
      if (outcome.status === "plan") activePlan = outcome.id;
      return { outcome, scope: null };
    }
    case "preview_page": {
      if (args.id !== activePlan) throw "This preview expired. Run the command again.";
      // The 26-file PDF move: steps 6 to 26, with one more name clash.
      const offset = Number(args.offset);
      const steps = Array.from({ length: 26 }, (_, step) => step >= 5 ? {
        step, kind: "move", primary: `scan-${String(step + 1).padStart(2, "0")}.pdf`, detail: "→ ~/Documents", skippable: true,
        ...(step === 17 ? { note: "“scan-18.pdf” already exists, so this becomes “scan-18 2.pdf”" } : {}),
      } : step === 2 ? { step, kind: "move", primary: "report.pdf", detail: "→ ~/Documents/report 2.pdf", note: "“report.pdf” already exists, so this becomes “report 2.pdf”" } : null);
      return steps.filter((r): r is NonNullable<typeof r> => !!r && r.step >= offset && (!args.onlyNotes || "note" in r));
    }
    case "confirm_plan":
      activePlan = null;
      if (!recoveryReviewed && mockScenario?.startsWith("recovery")) throw new Error("Open Settings → Scheduled, check the files, then choose Mark as reviewed.");
      if (pendingSchedule) {
        const { runNow, total = 3, intervalMs } = pendingSchedule;
        if (runNow) pendingSchedule.due = new Date(Date.now() + intervalMs!).toISOString();
        scheduled.push(pendingSchedule);
        const first = new Date(pendingSchedule.due).toLocaleString();
        const summary = runNow ? `Done. ${total} ${total === 1 ? "change" : "changes"}. Repeats ${pendingSchedule.repeat}; next run ${first}. Manage tasks in Settings → Scheduled.` : pendingSchedule.repeat ? `Repeats ${pendingSchedule.repeat}, first on ${first}. Manage tasks in Settings → Scheduled.` : `Scheduled for ${first}. Manage tasks in Settings → Scheduled.`;
        pendingSchedule = null;
        return { done: runNow ? total : 0, total, error: null, cancelled: false, undoable: !!runNow, summary } satisfies RunResult;
      }
      {
        const n = 26 - ((args.skip as number[] | undefined)?.length ?? 0);
        return { done: n, total: n, error: null, cancelled: false, undoable: true, summary: `Done. ${n} changes.` } satisfies RunResult;
      }
    case "get_settings":
      return settings;
    case "save_settings": {
      const incoming = args.settings as Settings;
      for (const s of incoming.shortcuts) {
        const total = s.commands.reduce((n, c) => n + utf8Bytes(c), 0);
        if (utf8Bytes(s.phrase) > MAX_COMMAND_BYTES || s.commands.length > 8 || total > MAX_COMMAND_BYTES) throw TOO_LONG;
      }
      settings = incoming;
      settings.shortcuts = settings.shortcuts
        .map((s) => ({ phrase: s.phrase.trim(), commands: s.commands.map((c) => c.trim()).filter(Boolean) }))
        .filter((s) => s.phrase || s.commands.length);
      if (settings.shortcuts.some((s) => !s.phrase)) throw "Give each shortcut a phrase to say, like “run my usuals”.";
      return settings;
    }
    case "report_wrong":
      return { count: ++reports, path: "~/Library/Application Support/voxi/reports.jsonl" };
    case "reports_summary":
      return { count: reports, path: "~/Library/Application Support/voxi/reports.jsonl" };
    case "clear_reports":
      reports = 0;
      return { count: 0, path: "~/Library/Application Support/voxi/reports.jsonl" };
    case "reveal_reports":
      return;
    case "teach_phrase": {
      const phrase = String(args.phrase);
      const command = String(args.command);
      if (utf8Bytes(phrase) > MAX_COMMAND_BYTES || utf8Bytes(command) > MAX_COMMAND_BYTES) throw TOO_LONG;
      settings = { ...settings, shortcuts: [...settings.shortcuts, { phrase, commands: [command] }] };
      return settings;
    }
    case "remember_folder": {
      const name = String(args.name).toLowerCase();
      settings = { ...settings, nicknames: [...settings.nicknames.filter((n) => n.name !== name), { name, path: String(args.path) }] };
      return settings;
    }
    case "request_folder_access": {
      if (mockScenario === "permission-reject" || (mockScenario === "permission-flaky" && permissionAttempts++ === 0))
        throw new Error("Mock: the folder-access request was rejected by the system.");
      if (mockScenario === "permission-denied")
        return [
          { name: "Desktop", path: "~/Desktop", allowed: false },
          { name: "Downloads", path: "~/Downloads", allowed: false },
          { name: "Documents", path: "~/Documents", allowed: false },
          { name: "Pictures", path: "~/Pictures", allowed: false },
        ];
      return [
        { name: "Desktop", path: "~/Desktop", allowed: true },
        { name: "Downloads", path: "~/Downloads", allowed: true },
        { name: "Documents", path: "~/Documents", allowed: false },
        { name: "Pictures", path: "~/Pictures", allowed: true },
      ];
    }
    case "open_privacy_settings":
      if (mockScenario === "settings-fail") throw new Error("Mock: System Settings could not be opened.");
      return;
    case "allowed_folders": {
      const shown = (p: string) => p.replace(/^\/Users\/you/, "~");
      const excluded = settings.excludedFolders.map(shown);
      const hidden = (p: string) => excluded.some((x) => p === x || p.startsWith(`${x}/`));
      const builtIn = ["~/Desktop", "~/Downloads", "~/Documents", "~/Pictures"].filter((p) => !hidden(p));
      return [...builtIn, ...settings.extraFolders.map(shown)];
    }
    default:
      return null;
  }
}
