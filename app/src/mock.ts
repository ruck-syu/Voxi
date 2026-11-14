// Browser-only stand-in for the Rust backend, so the window can be designed
// and reviewed with `npm run dev` outside Tauri. Never loaded inside the app.
import type { Outcome, RunResult, Settings, ScheduledTask } from "./api";

let settings: Settings = {
  hotkey: "Control+Alt+Space", extraFolders: [], startAtLogin: false,
  // ?welcome previews the first-run page.
  onboarded: !new URLSearchParams(location.search).has("welcome"),
  shortcuts: [{ phrase: "run my usuals", commands: ["move screenshots from today to Screenshots", "delete installers from Downloads"] }],
  nicknames: [{ name: "dev", path: "~/Documents/Dev" }],
};
let reports = 0;
let scheduled: ScheduledTask[] = [];
let pendingSchedule: ScheduledTask | null = null;

function plan(text: string, choices: Record<string, string> = {}): Outcome {
  const t = text.toLowerCase();
  if (t.includes("usuals"))
    return {
      status: "plan", id: 4, title: "Run my usuals: 2 commands", total: 3, isUndo: false,
      warnings: ["“delete installers from Downloads”: No files match in Downloads."],
      items: [
        { kind: "create_folder", primary: "Screenshots", detail: "new folder in ~/Desktop" },
        { kind: "move", primary: "Screenshot 2026-10-28 at 09.14.02.png", detail: "→ ~/Desktop/Screenshots" },
        { kind: "move", primary: "Screenshot 2026-10-28 at 11.40.19.png", detail: "→ ~/Desktop/Screenshots" },
      ],
    };
  if (t.includes("pdf"))
    return {
      status: "plan", id: 1, title: "Move 26 files to Documents", total: 26, isUndo: false,
      warnings: ["2 already in Documents, left alone."],
      items: [
        { kind: "move", primary: "Q3 board deck.pdf", detail: "→ ~/Documents" },
        { kind: "move", primary: "lease-renewal-2026.pdf", detail: "→ ~/Documents" },
        { kind: "move", primary: "report.pdf", detail: "→ ~/Documents/report 2.pdf", note: "“report.pdf” already exists, so this becomes “report 2.pdf”" },
        { kind: "move", primary: "tax-receipt-march.pdf", detail: "→ ~/Documents" },
        { kind: "move", primary: "W-2 2025.pdf", detail: "→ ~/Documents" },
      ],
    };
  if (t.includes("clean"))
    return {
      status: "plan", id: 2, title: "Move 3 files from Downloads to the Trash", total: 3, isUndo: false, warnings: [],
      items: [
        { kind: "trash", primary: "Zoom.pkg", detail: "→ Trash" },
        { kind: "trash", primary: "Figma-124.3.dmg", detail: "→ Trash" },
        { kind: "trash", primary: "old-export.csv", detail: "→ Trash" },
      ],
    };
  if (t.includes("claude") && choices.claude)
    return {
      status: "plan", id: 3, title: `Move ${choices.claude.split("/").pop()} from Downloads to the Trash`, total: 1, isUndo: false, warnings: [],
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
    case "cancel_plan": pendingSchedule = null; return;
    case "search_progress": return 0;
    case "result_scope": return null;
    case "clear_result_scope": return;
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
      const text = String(args.text);
      const repeat = text.match(/\s+every\s+(\d+)\s+(days?|hours?)$/i);
      if (repeat) {
        const outcome = plan(text.slice(0, repeat.index), args.choices as Record<string, string>);
        if (outcome.status !== "plan") return { outcome, scope: null };
        const every = `every ${repeat[1]} ${repeat[2]}`;
        const due = new Date(Date.now() + Number(repeat[1]) * (repeat[2].startsWith("day") ? 86400000 : 3600000));
        pendingSchedule = { id: scheduled.length + 1, command: text, due: due.toISOString(), status: "pending", summary: "", repeat: every };
        return { outcome: { ...outcome, scheduledFor: due.toISOString(), repeats: every, warnings: [`Repeats ${every}, first on ${due.toLocaleString()}. Each run changes whatever matches at that time, without a preview, and can be undone from History.`, ...outcome.warnings] }, scope: null };
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
      return { outcome, scope: null };
    }
    case "confirm_plan":
      if (pendingSchedule) {
        scheduled.push(pendingSchedule);
        const first = new Date(pendingSchedule.due).toLocaleString();
        const summary = pendingSchedule.repeat ? `Repeats ${pendingSchedule.repeat}, first on ${first}. Manage tasks in Settings → Scheduled.` : `Scheduled for ${first}. Manage tasks in Settings → Scheduled.`;
        pendingSchedule = null;
        return { done: 0, total: 3, error: null, undoable: false, summary } satisfies RunResult;
      }
      return { done: 26, total: 26, error: null, undoable: true, summary: "Done. 26 changes." } satisfies RunResult;
    case "get_settings":
      return settings;
    case "save_settings":
      settings = args.settings as Settings;
      settings.shortcuts = settings.shortcuts
        .map((s) => ({ phrase: s.phrase.trim(), commands: s.commands.map((c) => c.trim()).filter(Boolean) }))
        .filter((s) => s.phrase || s.commands.length);
      if (settings.shortcuts.some((s) => !s.phrase)) throw "Give each shortcut a phrase to say, like “run my usuals”.";
      return settings;
    case "report_wrong":
      return { count: ++reports, path: "~/Library/Application Support/voxi/reports.jsonl" };
    case "reports_summary":
      return { count: reports, path: "~/Library/Application Support/voxi/reports.jsonl" };
    case "clear_reports":
      reports = 0;
      return { count: 0, path: "~/Library/Application Support/voxi/reports.jsonl" };
    case "reveal_reports":
      return;
    case "teach_phrase":
      settings = { ...settings, shortcuts: [...settings.shortcuts, { phrase: String(args.phrase), commands: [String(args.command)] }] };
      return settings;
    case "remember_folder": {
      const name = String(args.name).toLowerCase();
      settings = { ...settings, nicknames: [...settings.nicknames.filter((n) => n.name !== name), { name, path: String(args.path) }] };
      return settings;
    }
    case "request_folder_access":
      return [
        { name: "Desktop", path: "~/Desktop", allowed: true },
        { name: "Downloads", path: "~/Downloads", allowed: true },
        { name: "Documents", path: "~/Documents", allowed: false },
        { name: "Pictures", path: "~/Pictures", allowed: true },
      ];
    case "allowed_folders":
      return ["~/Desktop", "~/Downloads", "~/Documents", "~/Pictures"];
    default:
      return null;
  }
}
