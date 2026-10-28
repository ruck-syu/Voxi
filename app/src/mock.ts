// Browser-only stand-in for the Rust backend, so the window can be designed
// and reviewed with `npm run dev` outside Tauri. Never loaded inside the app.
import type { Outcome, RunResult, Settings } from "./api";

let settings: Settings = {
  hotkey: "Control+Alt+Space", extraFolders: [], startAtLogin: false,
  // ?welcome previews the first-run page.
  onboarded: !new URLSearchParams(location.search).has("welcome"),
  shortcuts: [{ phrase: "run my usuals", commands: ["move screenshots from today to Screenshots", "delete installers from Downloads"] }],
};

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
      status: "ambiguous", name: "claude",
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
    case "plan_command":
      return plan(String(args.text), args.choices as Record<string, string>);
    case "confirm_plan":
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
