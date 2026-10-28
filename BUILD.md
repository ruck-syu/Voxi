# Voxi: Build Plan (Mac v1)

## 1. Overview

Voxi is a small menu-bar app for macOS that organizes files from a text command. Press a global hotkey and a floating window opens with its text box already focused. The user either types a command or dictates it with Wispr Flow, which fills the same text box. Pressing Enter parses the command into a list of file actions, shows a preview, and runs them only after the user confirms.

Voxi does not do speech-to-text. Wispr Flow (or the keyboard) is the only input method.

This project is an entry for **No Hands: The Wispr Challenge** (WeMakeDevs x Wispr Flow). Projects submitted to the challenge enter a daily draw for $100 each, with ten prizes in total.

"Voxi" is a working name. Confirm it, and check that the GitHub name and domain are free, before the first public release.

**Version 1 is Mac only.** Windows support comes after v1 ships.

## 2. Goals and non-goals

**Goals (v1)**
- Run in the menu bar with no Dock icon.
- Global hotkey toggles a floating command window, with the text box focused on open.
- The text box accepts typing and Wispr Flow dictation equally.
- Parse commands with a rule-based parser covering the command catalog in section 4.
- Show a preview of every change. Nothing changes until the user confirms.
- Undo the last change.
- Public GitHub repo, a downloadable signed `.dmg`, and a landing page that links to both.

**Non-goals (v1)**
- Speech-to-text inside the app. No Whisper, no microphone capture, no cloud transcription.
- LLM-based parsing. It can come after v1 as an opt-in fallback.
- Windows, Linux, or mobile builds.
- Hard deletes. Anything that removes files moves them to `_to_delete/`.
- Any integration with Wispr Flow beyond the text box.

## 3. Tech stack

| Layer | Choice | Reason |
|---|---|---|
| App shell | Tauri v2 (Rust backend, web frontend) | Small install size, native tray and window support |
| Frontend | TypeScript + React or Svelte | Builds the floating window and the landing page |
| Command parsing | Rule-based patterns (regex with named groups) | Predictable, easy to unit test |
| File operations | Rust `std::fs` through Tauri commands | Native paths and permissions |
| Image work | macOS built-in `sips` | Already on every Mac: convert and resize |
| PDF work | PDFKit through a small Swift helper, or `pdfunite` from poppler | Merge, split, and extract pages |
| Archives | macOS built-in `zip` and `unzip` | Already on every Mac |
| Hotkey | Tauri global shortcut plugin | Toggles the floating window |
| Website | Static site (Vite or Next.js static export), hosted on Vercel or GitHub Pages | Free hosting, simple deploys |

## 4. How it works

```
[Global hotkey] -> [Floating window opens, text box focused]
      -> [User types or dictates with Wispr Flow into the box]
      -> [Enter] -> [Rule-based parser] -> [Action plan]
      -> [Preview list in window] -> (user confirms) -> [Actions run in Rust]
      -> [Result shown, log written, undo available]
```

**Floating window behavior**
- Borderless, centered on the screen with the cursor, always on top, hidden by default.
- The hotkey shows it and focuses the text box. Pressing the hotkey again or pressing Escape hides it.
- Enter parses the text. Shift+Enter inserts a newline, so multi-line commands work.
- Wispr Flow's hotkey dictates into whatever field has focus. Because the box is focused when the window opens, the user only needs Voxi's hotkey and then Wispr's hotkey.

**Folder names and filters**
- Folder names resolve to the user's real folders: Desktop, Downloads, Documents, Pictures, plus Screenshots (a subfolder of Desktop, if it exists).
- Type filters: `screenshots`, `images`, `pictures`, `pdfs`, `videos`, `documents`, `archives`, `files` (any).
- `screenshots` matches files whose names start with "Screenshot" and whose type is image.
- Date filters: `today`, `yesterday`, `last N days`, `older than N days`.
- Size filters: `larger than N MB`, `smaller than N MB`.
- Name filters: `named X` or `containing X`.

**Command catalog (v1)**

Each row is one parser pattern and one test. Examples use the words as a user would say them.

| # | Category | Example command | Action kind |
|---|---|---|---|
| 1 | Move | "move report.pdf to Documents" | `move` |
| 2 | Move | "move screenshots from today to Screenshots" | `batch` of `move` |
| 3 | Move | "move all PDFs from Downloads to Documents" | `batch` of `move` |
| 4 | Move | "move files larger than 100 MB from Downloads to External" | `batch` of `move` |
| 5 | Move | "move files containing invoice from Downloads to Invoices" | `batch` of `move` |
| 6 | Copy | "copy notes.txt to Desktop" | `copy` |
| 7 | Copy | "copy all images from Pictures to Backup" | `batch` of `copy` |
| 8 | Rename | "rename report.pdf to Q3-report.pdf" | `rename` |
| 9 | Rename | "add prefix Invoice- to files named bill in Downloads" | `batch` of `rename` |
| 10 | Rename | "add date prefix to files in Screenshots" | `batch` of `rename` |
| 11 | Rename | "replace spaces with dashes in files in Downloads" | `batch` of `rename` |
| 12 | Rename | "replace draft with final in names in Documents" | `batch` of `rename` |
| 13 | Rename | "number images in Trip as trip-001, trip-002" | `batch` of `rename` |
| 14 | Rename | "lowercase file names in Downloads" | `batch` of `rename` |
| 15 | Folders | "create folder Invoices on Desktop" | `create_folder` |
| 16 | Folders | "create folders 2026, 2027 in Documents" | `batch` of `create_folder` |
| 17 | Sort | "sort Downloads by type" | `batch` of `move` into type folders |
| 18 | Sort | "sort Screenshots by month" | `batch` of `create_folder` and `move` |
| 19 | Images | "convert photo.heic to png" | `convert_image` |
| 20 | Images | "convert all HEIC in Pictures to jpg" | `batch` of `convert_image` |
| 21 | Images | "resize photo.jpg to 1200 pixels wide" | `resize_image` |
| 22 | Images | "resize all images in Trip to 1600 pixels wide" | `batch` of `resize_image` |
| 23 | PDF | "merge PDFs in Documents named scan into scan-all.pdf" | `merge_pdf` |
| 24 | PDF | "merge all PDFs in Downloads into combined.pdf" | `merge_pdf` |
| 25 | PDF | "split report.pdf into pages" | `split_pdf` |
| 26 | PDF | "extract pages 2 to 5 from report.pdf into excerpt.pdf" | `extract_pages` |
| 27 | PDF | "make a PDF from images in Scans named receipts" | `images_to_pdf` |
| 28 | Archives | "zip Project into Project.zip" | `zip` |
| 29 | Archives | "unzip archive.zip into Downloads/archive" | `unzip` |
| 30 | Cleanup | "clean up Downloads older than 90 days" | `batch` of `trash` |
| 31 | Cleanup | "move installers from Downloads to Trash folder" | `batch` of `trash` |
| 32 | Cleanup | "delete screenshots older than 30 days" | `batch` of `trash` (never a hard delete) |
| 33 | Find | "list files in Downloads larger than 500 MB" | `list` (read-only) |
| 34 | Find | "list files older than 1 year in Documents" | `list` (read-only) |
| 35 | Find | "show size of Downloads" | `list` (read-only) |
| 36 | Find | "count PDFs in Documents" | `list` (read-only) |
| 37 | Find | "list duplicate files in Downloads" | `list` (read-only, by name and size) |
| 38 | Open | "open Downloads in Finder" | `open_in_finder` |
| 39 | Open | "reveal report.pdf in Finder" | `open_in_finder` |
| 40 | Undo | "undo last" | `undo` |
| 41 | Undo | "show recent actions" | `list` (read-only, from the log) |

**Parser rules**
- Matching is case-insensitive, and extra filler words ("the", "all", "please") are ignored.
- Each pattern is a named regex. If two patterns match, the more specific one wins. If none match, show "Didn't understand."
- `batch` previews show the total count and the first 20 items.
- Ambiguous names (two files with the same name in allowed folders) are listed in the preview for the user to choose from. Voxi does not pick one silently.

**Action type**
```ts
type Filter = {
  type?: "screenshots" | "images" | "pdfs" | "videos" | "documents" | "archives" | "files";
  olderThanDays?: number;
  newerThanDays?: number;
  largerThanMB?: number;
  smallerThanMB?: number;
  nameContains?: string;
};

type Action =
  // Files
  | { kind: "move"; from: string; to: string }
  | { kind: "copy"; from: string; to: string }
  | { kind: "rename"; path: string; newName: string }
  | { kind: "trash"; path: string }                      // moves into _to_delete/
  | { kind: "create_folder"; path: string }
  // Bulk: expanded from a Filter at parse time, previewed as one step
  | { kind: "batch"; actions: Action[] }
  // Images
  | { kind: "convert_image"; path: string; format: "png" | "jpg" | "webp" | "heic" }
  | { kind: "resize_image"; path: string; maxWidthPx: number }
  | { kind: "images_to_pdf"; paths: string[]; output: string }
  // PDF
  | { kind: "merge_pdf"; paths: string[]; output: string }
  | { kind: "split_pdf"; path: string }
  | { kind: "extract_pages"; path: string; from: number; to: number; output: string }
  // Archives
  | { kind: "zip"; path: string; output: string }
  | { kind: "unzip"; path: string; destination: string }
  // Read-only (no preview needed, no changes)
  | { kind: "list"; folder: string; filter: Filter }
  | { kind: "open_in_finder"; path: string }
  // Meta
  | { kind: "undo" };
```

## 5. Safety rules (must hold in every build)

1. Show the full action list before any change. Read-only actions (`list`, `open_in_finder`) may run without a preview.
2. Only touch paths inside the allowed folders in config.
3. No hard deletes. Anything that removes a file, including "delete" and "clean up", becomes a move to `_to_delete/`.
4. Reject paths containing `..` after normalization.
5. If the command matches no pattern, show "Didn't understand" with the raw text. Do not guess.
6. Log every executed action with a timestamp and the inverse action, so `undo` can reverse the last change.
7. `undo` is itself previewed before it runs.
8. Overwrites never happen silently. If a target name already exists, the preview shows a rename suffix (for example `report 2.pdf`) and the user can change it.

## 6. Mac-specific setup

- **Permissions:** Accessibility or Input Monitoring (for the global hotkey), and file access to Downloads, Desktop, Documents, and Pictures. Voxi needs no microphone permission, since it doesn't record audio. Request each permission at first run with a short explanation in the UI.
- **Menu bar only:** Set `LSUIElement` in `Info.plist` so the app has no Dock icon. The tray menu offers Show, Settings, and Quit.
- **Focus:** On show, call focus on the window and then on the text box. macOS may not focus a window opened in the background, so do both.
- **Hotkey conflicts:** Pick a Voxi hotkey that doesn't clash with Wispr Flow's default shortcut. Make the hotkey configurable in Settings.
- **Start at login:** Optional setting, off by default, using the login item API.
- **Signing and notarization:** Sign with a Developer ID certificate and notarize with `notarytool` before publishing the `.dmg`. Without this, Gatekeeper blocks the app on other users' machines.

## 7. Frontend and website work: use the frontend-design skill

Before writing any UI or landing-page code, load the `frontend-design` skill with the Skill tool. It gives guidance on intentional visual design and typography, so the result doesn't look like a template.

Apply it to:
- The floating window: the input box, the preview list of actions with counts for batches, status states (ready, parsing, awaiting confirmation, done, error), and the result view with undo.
- The landing page: hero section, a short demo clip or GIF showing the hotkey, Wispr dictation, and the preview, plus a feature list, install steps, and links.
- Consistent typography and color tokens shared between the app and the site.

## 8. Website

**Purpose:** Explain the app and link to the GitHub repo and the download.

**Pages (v1)**
- Home: one-line pitch, demo clip, three feature points, two buttons: "Download for Mac" and "View on GitHub."
- Commands: the command catalog from section 4, in plain language, so people can see what Voxi understands.
- Install: the Mac steps, including the Gatekeeper and permission steps.
- Footer: link to the repo, license, and the challenge page.

**Links**
- "View on GitHub" points to the public repo.
- "Download for Mac" points to the latest GitHub Release asset (`.dmg`), so the site doesn't need updating for each release.

**Hosting**
- Deploy the site to Vercel or GitHub Pages from its own folder or repo.
- Add a custom domain if you have one.

## 9. Repository layout

```
voxi/
  app/                  # Tauri app (src-tauri/ for Rust, src/ for frontend)
  app/src-tauri/parse/  # Parser patterns and tests, one file per category
  website/              # Landing page (static site)
  docs/                 # Demo media, screenshots
  BUILD.md              # This file
  README.md             # Install, usage, command list, license, challenge note
  LICENSE
```

## 10. Milestones

1. **Skeleton:** Tauri app runs, menu-bar icon shows, floating window opens and closes.
2. **Hotkey and focus:** Global hotkey shows the window with the text box focused. Escape and the hotkey both hide it.
3. **Parser:** Command catalog from section 4, one unit test per row, plus tests that unrecognized input returns "Didn't understand."
4. **Preview and confirm:** Action list with batch counts, confirm, cancel, result view.
5. **File ops and safety:** Allowed folders, `_to_delete/` handling, overwrite suffixes, logging with inverse actions, undo.
6. **Image and PDF tools:** Wire `sips`, the PDF helper, and `zip` or `unzip` into the action runner, with tests on sample files.
7. **Wispr check:** Confirm that Wispr Flow dictates into the window's text box, and that the hotkey doesn't clash with Wispr's default shortcut.
8. **Permissions and onboarding:** First-run permission requests and a short explainer.
9. **Packaging:** Signed and notarized `.dmg`, GitHub Release.
10. **Website:** Built with the frontend-design skill, deployed, linked to the repo and the release.
11. **Demo and submission:** Record a short demo, write the README, submit to the challenge.

## 11. Open questions

- Does the challenge expect Wispr Flow to be part of the submission? Check the challenge Overview page first.
- Which default hotkey should Voxi use so it doesn't clash with Wispr Flow's?
- Confirm the name "Voxi" and check the GitHub name and domain.
- Should the "clean up" and "delete" commands ask for a retention period, or use a fixed `_to_delete/` cleanup later?
- License choice (MIT or Apache-2.0 are common for this kind of project).
- After v1: add an opt-in LLM fallback for commands the rule-based parser doesn't match?

## 12. Definition of done (v1)

- A user can install the signed `.dmg`, grant permissions, and run any command in the catalog end to end.
- The hotkey opens the window with the text box focused, and Wispr Flow dictation fills it.
- Every change shows in the preview and requires confirmation. Read-only commands run directly.
- No file is deleted. Nothing runs outside the allowed folders.
- `undo` reverses the last change.
- The GitHub repo has a README with the command list, a license, and the release link.
- The website is live, uses the frontend-design guidance, and links to the repo and download.
