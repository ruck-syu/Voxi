# Voxi

A menu-bar app for macOS that organizes files from a typed or dictated command.
Press **⌃⌥Space**, type (or dictate with Wispr Flow) something like
“move all PDFs from Downloads to Documents”, check the preview, and press Enter to run it.

- Nothing changes until you confirm the preview with Enter.
- Voxi only touches Desktop, Downloads, Documents, and Pictures, plus folders you add in Settings.
- Nothing is ever deleted outright: “delete” and “clean up” move files to the macOS Trash, and `undo last` takes them back out.
- Overwrites never happen; a clashing name becomes `report 2.pdf`, and the preview says so.
- `undo last` reverses the last change (also previewed first).

See [BUILD.md](BUILD.md) for the full command catalog and plan.

## Install

Download the `.dmg` from the [latest release](https://github.com/ruck-syu/Voxi/releases/latest),
open it, and drag Voxi to Applications.

Voxi isn't notarized yet, so the first time macOS will say it can't check it for
malicious software. Right-click Voxi in Applications and choose **Open**, then
**Open** again. After that it opens normally.

## Updates

Every push to `main` builds a new release (`.github/workflows/release.yml`).
In Voxi, open **Settings → Check for updates**: it downloads the latest
release with a progress bar, checks its signature, installs it, and restarts.

## Website

`website/` is a static site (no build step) for Vercel: import this repo in
Vercel and set **Root Directory** to `website`. Pushes that only touch the
website or docs don't trigger an app release.

## Develop

```bash
cd app
npm install
npm run tauri dev      # runs the menu-bar app
```

Parser, planner, and executor tests (one test per catalog row, plus safety tests
that run real moves, sips, ditto, and PDFKit in a temp folder):

```bash
cd app/src-tauri && cargo test
```

`npm run dev` alone serves the window in a browser with a mock backend
(`app/src/mock.ts`), for design work.

## Layout

- `app/src-tauri/src/parse/` — rule-based parser, one file per command category
- `app/src-tauri/src/plan.rs` — resolves names, expands filters, checks paths, builds the preview
- `app/src-tauri/src/exec.rs` — runs a confirmed plan and records the inverse of each step
- `app/src-tauri/src/pdf.rs` — merge, split, extract, images to PDF via PDFKit
- `app/src/` — the floating command window (React)
