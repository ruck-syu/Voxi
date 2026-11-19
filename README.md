<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/readme/banner-dark.png" />
    <img src="docs/readme/banner-light.png" width="640" alt="Voxi: a smiling face whose smile is a sound wave, next to the words: organize your Mac's files by saying what you want." />
  </picture>
</p>

<p align="center">
  <a href="https://github.com/ruck-syu/Voxi/releases/latest"><b>Download for Mac</b></a>
  &nbsp;&nbsp;|&nbsp;&nbsp;
  <a href="website/commands.html">Commands</a>
  &nbsp;&nbsp;|&nbsp;&nbsp;
  <a href="#install">Install</a>
  <br />
  <sub>Free and open source. macOS 12 or later, Apple silicon and Intel.</sub>
</p>

---

Press <kbd>⌃⌥Space</kbd> anywhere. A small window opens with the cursor ready.
Type a command or dictate with an app such as [Wispr Flow](https://wisprflow.ai) or [Hex by Anomaly](https://anoma.ly/hex/):

> move all PDFs from Downloads to Documents

Voxi shows exactly what will happen, file by file. Press <kbd>Enter</kbd> to
run it, or <kbd>Esc</kbd> to back out. Changed your mind afterwards?
Say **undo last**.

## What it can do

| You say | Voxi |
|---|---|
| move screenshots from today to Screenshots | Files today's screenshots, wherever they are |
| clean up Downloads older than 90 days | Moves old files to the Trash |
| rename report.pdf to Q3-report.pdf | Renames one file |
| create a folder in Downloads named PDFS | Creates the PDFS folder in Downloads |
| replace spaces with dashes in files in Downloads | Renames in bulk |
| sort Downloads by type | Groups files into Images, PDFs, Videos, and more |
| convert all HEIC in Pictures to jpg | Writes JPG copies |
| merge PDFs in Documents named scan into scan-all.pdf | Combines PDFs |
| zip Project into Project.zip | Compresses a folder |
| list files in Downloads larger than 500 MB | Shows numbered matches, changes nothing |
| delete 4 | Trashes row 4 of the list on screen |
| move PDFs from Downloads to Documents at 9 pm | Runs once, tonight |
| delete dmg files from Downloads every 3 days | Repeats on a schedule |
| run my usuals | Runs a shortcut you saved in Settings |

There are 27 kinds of commands, with filters for type, date (including months,
like "in August"), size, and name.
The full list is on the [Commands page](website/commands.html).

### How searching works

File commands search the folder you name and its subfolders, up to 12 levels
deep. This applies to groups of files and to individual file or folder names;
multiple matching names prompt you to choose. Hidden folders, app bundles,
symlinked folders, and `_to_delete` are skipped. Moves and copies are the
exception: they take only the files directly in the folder you name, so files
kept in its subfolders stay put. Renames and image edits keep files in their folders.

Searches reuse one folder scan per command. The window shows progress, and you
can cancel with Esc or Cancel. If a scan reaches its depth, entry, or time limit,
or cannot read a folder, commands that change files stop with an explanation;
read-only results are marked as partial. Searches are limited to 50,000 entries
per folder tree and 10 seconds; the Spotlight fallback has a separate 2-second
limit within that search deadline.

Spaces, underscores, and hyphens are equivalent in name filters. Individual
filename searches rank exact and normalized names before word matches and typo
suggestions. Loose or typo matches always ask you to choose, even if there is
only one suggestion; broad file filters never automatically expand to typos.

### Working with a list

After a list command, follow-up commands use that exact set of files. For example,
`list files in Downloads`, then `delete the last file` selects the last displayed
row; `move PDFs to Documents` selects only PDFs from that list.
Each displayed item is numbered: `delete 4` or `move 5 to Desktop` targets that
row. Spoken numbers such as “four” work too; quote a literal numeric filename.
Numbers stay attached to their original files until another list is displayed.
New files added after listing aren't included. Naming another source folder or an unrelated file
starts a fresh search. The active scope is shown above the results; use **Clear
scope** to reset it. Closing Voxi or opening Settings also ends the scope.
Changes still require a preview and confirmation. If a listed file disappears or
is replaced, run the list command again. Positional references use the displayed
rows (up to 200); group commands can use every file in a complete list.

### Names, quotes, and sizes

Without a list, commands need a filename or a specific group. Quote literal names
containing command words, for example `move "Notes to Self.txt" to Documents`.
Quoted names also preserve words such as “one”. Size filters require units;
contradictory or repeated bounds are refused rather than silently discarded.
Use “undo last” for the newest change; selecting an earlier move or delete by
its operation type isn't supported.

## Dictation

Use an external dictation app such as [Wispr Flow](https://wisprflow.ai) or [Hex by Anomaly](https://anoma.ly/hex/).
Hex downloads a speech model once, then transcribes locally and pastes into the
focused app. Its Mac download requires Apple silicon and macOS 15 or later.

Open Voxi, focus the command bar, dictate with Wispr Flow or Hex, then press Enter to review
the command. Other dictation apps work too. Voxi has no built-in recording or
speech recognition and doesn't request microphone or speech permissions.

## Safe by design

- **Nothing changes until you press Enter.** Every command is previewed first.
- **Nothing is erased.** "Delete" and "clean up" move files to the Trash.
- **Nothing is overwritten.** A clashing name becomes `report 2.pdf`, and the preview says so. Destinations are claimed atomically, so a file created after the preview cannot be clobbered.
- **Everything can be undone.** "Undo last" reverses the newest change as one step, after its own preview. If a chain or shortcut fails partway, the steps that completed stay undoable when their history entry was saved, and Voxi reports the failure.
- **Voxi stays in its lane.** It only touches Desktop, Downloads, Documents, Pictures, and folders you add in Settings.
- **No guessing.** When a name matches several files, Voxi lists them and asks which one; say a number to answer.
- **Commands are bounded.** A command may be at most 16 KiB of UTF-8 text. A saved shortcut holds at most 8 commands whose combined text is at most 16 KiB. Larger input is refused before Voxi plans anything.

There are limits worth knowing. Every file operation is written to a journal
before it runs. If Voxi is killed partway through, the steps it had already
recorded as complete are recovered into History; the interrupted step is **never
replayed**, and its result must be checked by hand. If Voxi stops before the
history entry was saved, that step has no durable undo. Settings → Scheduled
shows a recovery notice with **Mark as reviewed**: reviewing archives the journal
evidence and keeps any recovered undo, but does not retry the operation. History
keeps the 200 most recent changes, and "undo last" reaches the newest one only.
If settings, history, or schedules are corrupt, Voxi preserves a
`.corrupt-<timestamp>` backup, shows a notice, and starts with defaults; later
saves proceed normally, with the original kept in the backup. Saves are blocked
— and the original left untouched — only when that backup could not be written,
the file could not be read, or it declares a newer version from a later Voxi.
This recovery evidence is kept separate from the private diagnostics described
below.

## Install

With [Homebrew](https://brew.sh), run this, then continue from step 2:

```bash
brew install --cask ruck-syu/tap/voxi
```

Or by hand:

1. Download the `.dmg` from the [latest release](https://github.com/ruck-syu/Voxi/releases/latest), open it, and drag Voxi to Applications.
2. Open Voxi. Releases are signed with Voxi's own certificate but are not notarized by Apple yet, so macOS blocks the first launch:
   - **macOS 15 or later:** open System Settings → Privacy & Security, scroll down, and click **Open Anyway** next to Voxi.
   - **macOS 14 or earlier:** right-click Voxi in Applications, choose **Open**, then **Open** again.

   macOS remembers this, so you only do it once.
3. Voxi opens a welcome page. Click **Allow access** and approve Desktop, Downloads, Documents, and Pictures so commands never stop to ask. These are ordinary **Files and Folders** grants; that is all Voxi needs by default. If you ask Voxi to work in a protected location outside those folders — such as another app's container — and the folder grant is not enough, enable **Full Disk Access** for Voxi under System Settings → Privacy & Security, and only for those locations. Voxi does not need Full Disk Access by default. If a grant is missing later, the folder-access page offers **Check again** and **Open System Settings**; a failed request offers **Retry**.

Voxi lives in the menu bar as a small face. Click it or press <kbd>⌃⌥Space</kbd>.

## Updates

Open Settings → General → **Check for updates**. Voxi downloads the newest
release with a progress bar, shows what's new, checks the update's signature,
installs it, and restarts.

## Chaining commands

Join up to eight folder-creation, move, copy, rename, or trash commands with
`and`, `then`, `and then`, or a semicolon. Voxi previews the complete chain,
asks for one confirmation, and records it as one change for **undo last**:

> create a folder in Downloads called PDFS and move PDFs into it

Here, `it` means the single folder created earlier in the chain. `that folder`
and `the new folder` also work. If the move or copy doesn't name a source,
it uses the parent of the created folder, Downloads in this example. An explicit
source always wins. Multiple created folders require an explicit destination.
Quotes keep command words and separators inside names literal.

Each step must be understood and plan successfully before a preview is offered.
Steps that reuse files from an earlier step must be run separately; creating a
folder and using it as a destination is supported. Clear any previous result
list first. A time or repeat at the end schedules the whole chain, for example
`create a folder in Downloads called PDFS and move PDFs into it everyday`.
`everyday`, `every day`, and `daily` all mean once a day. Without an explicit
time, the first run is one day from now; add `at 9 pm` to choose a clock time.
Repeating chains plan again at each run, reusing existing folders and picking
up newly matching files. Read-only commands, Finder actions, and undo run separately.

## Scheduled and repeating commands

Add a time, a delay, or a repeat to the end of a file command, review the
preview, then choose **Schedule task**:

- `move screenshots from Desktop to Screenshots folder at 9:40 pm`
- `move PDFs from Downloads to Documents after 3 hours`
- `clean up Downloads older than 90 days in 30 minutes`
- `delete dmg files from Downloads every 3 days`
- `sort Downloads by type every Monday at 8 am`

Clock times use your Mac's local timezone. A time that has passed means tomorrow;
you can also say `at 9:40 pm tomorrow` or use a 24-hour time such as `at 21:40`.
Delays accept whole hours or minutes, including spoken numbers through ninety-nine,
such as “three” or “twenty-one”.

Repeats accept `every 3 days`, `every other week`, `every Monday`, `daily at 9 pm`,
`hourly`, and `weekly`. A time of day works with days and weeks; a weekday without
a time runs at 9 am. Repeats are at least 15 minutes apart.

**One-time tasks** change only the files in their approved preview, excluding
files added later. Changed or missing source files stop the task.
**Repeating tasks** without a time of day run the approved preview immediately on
confirmation, then repeat at the chosen interval; tasks with a time of day first
run at the next occurrence of that time. Later runs plan the saved command again
and act on whatever matches then, without a new preview. A repeating task can't
use a previous list as its scope; name the files or folder instead.

Settings → Scheduled lists tasks, their results, and a cancellation button.
Tasks survive restarting Voxi. Keep Voxi running for on-time execution; if it is
closed or your Mac is asleep, an overdue task runs once when Voxi next runs or
your Mac wakes. No task wakes the Mac or launches Voxi.

Destination collisions never overwrite files, and every completed run enters
History, so `undo last` can reverse it. An interrupted **one-time** task is
marked failed; an interrupted **repeating** task skips the missed run and
schedules its next one. Voxi never replays a partially finished operation. In
either case a recovery notice in Settings → Scheduled lists the files to check;
choose **Mark as reviewed** once you have inspected them. That archives the
journal evidence and keeps any recovered undo, but does not retry the operation.
Undo, read-only, and Finder commands cannot be scheduled. If saved schedules
cannot be read, Voxi preserves a backup and shows a recovery notice in Settings
→ Scheduled. A storage error disables scheduling for that session while the rest
of the app remains available. History keeps the 200 most recent changes.

## Shortcuts

Save a phrase and a list of commands in Settings → Shortcuts. Saying the
phrase runs them all as one preview with one undo. "Run my usual" and
"run my usuals" both work. A shortcut can also hold a single read-only
command, like "list downloads", which runs as if you typed it; read-only
commands can't be mixed with commands that change files.

## Learning on your Mac

Voxi's parser is rule-based and runs locally. It learns from you without
sending anything anywhere:

- **Teach a phrase.** If Voxi doesn't understand a sentence and you rephrase it
  within a few minutes, it offers to run the working command the next time you
  say the original. Taught phrases appear under Settings → Shortcuts.
- **Folder nicknames.** When a name matches several folders and you pick one,
  Voxi offers to remember that choice.
- **Wrong-result reports.** Mark a result as wrong and Voxi saves a readable
  JSON line on this Mac. Show or clear them from Settings. Developers can turn
  them into parser test candidates with `scripts/feedback-to-corpus.mjs`.

## Privacy and diagnostics

Voxi keeps three small local log files to help diagnose startup, storage,
scheduler, login-item, and update failures. They live in
`~/Library/Application Support/dev.voxi.app/diagnostics/` as
`diagnostics.0.log` through `diagnostics.2.log`. The folder is `0700` and the
files are `0600`, readable only by your macOS account.

> Private operational diagnostics: three files (`diagnostics.0.log` through
> `diagnostics.2.log`), each at most 1 MiB, oldest discarded on rotation.
> Retention is size-based, with no age guarantee. No telemetry. History and
> opt-in feedback are separate stores with their existing retention policies.
> Producers never perform disk IO or wait for the writer: a full queue drops
> events. IO failure disables this session's sink with at most one fixed stderr
> note. Queued events may be lost on crash/exit; this is not a recovery journal.

Each line contains only a stable error code, a stage, an operation ID, counts,
and (for storage failures) an I/O error kind and OS number. No command text,
folder names, filenames, paths, notes, or model text are ever written, and
nothing is uploaded. Command text and paths stay in local History, which is a
separate store with the 200-entry retention described above.

## Develop

Voxi is a [Tauri](https://tauri.app) app: Rust for parsing and file work, React
for the window.

```bash
cd app && npm install
```

```bash
cd app && npm run tauri dev
```

Development builds run as "Voxi Dev" (`dev.voxi.app.dev`), with their own
settings, history, schedules and login item, so they never touch an installed
Voxi on the same Mac. Give the dev build a different hotkey if both run at once.

Tests cover one sentence per catalog row, safety rules, undo, and real runs of
`sips`, `ditto`, and PDFKit in a temporary folder:

```bash
cd app/src-tauri && cargo test --lib
```

The parser also runs a corpus of typed commands, one per line, in
`app/src-tauri/src/parse/corpus/`, and every line must pass. Held-out sets in
`app/src-tauri/eval/` measure how the parser handles phrasing it wasn't tuned
on. Run one with `HELDOUT=heldout-3 cargo test --lib heldout -- --ignored --nocapture`.
A set stops measuring accuracy once it has been used to fix something, so write
a new one before changing the parser.

`npm run dev` alone serves the window in a browser with a stand-in backend
(`app/src/mock.ts`), useful for design work.

To launch the native development app with the trained structured-plan v4c model:

```bash
cd app && npm run dev:model
```

This opt-in build uses the local exported package at
`~/.local/share/voxi/experiments/plan-model-v4c-2026-11-17/native-model`;
set `VOXI_PLAN_MODEL_DIR` to use a different location for the same verified
package. The loader checks all three file hashes. Weights stay outside Git.
Inference runs locally on a worker and interprets supported file changes,
chained requests and schedules before the rule parser. Explicit hard refusals,
read-only/other command kinds, shortcuts and active result lists keep their
existing paths. A model refusal falls back to the rules. Invalid model output,
missing/changed literal references and cancellation create no model-derived plan.
Every interpreted request still uses normal path validation, preview and
confirmation. Repeating model tasks persist the approved typed intent and plan
matching files again without interpreting the sentence anew. The preview marks
model interpretations so source folders and schedule can be checked.
The released product is **rule-only**. Release builds are compiled with
`--no-default-features`, so no model runtime or weights can enter a published app
regardless of the environment. This path exists only as an experimental
development opt-in via the `tiny-model` Cargo feature together with the
`VOXI_PLAN_MODEL_DIR` or `VOXI_TINY_MODEL_DIR` environment variables; neither
weights nor model accuracy are release requirements. With those variables unset,
the feature-enabled build produces the same public outcomes as the rule-only
build. This model was enabled at the user's request despite the documented
evaluation errors; see the
[training report](docs/testing/plan-model-v4-2026-11-17/REPORT.md).

Run the model integration smoke test against the real local package:

```bash
cd app/src-tauri
VOXI_PLAN_MODEL_DIR="$HOME/.local/share/voxi/experiments/plan-model-v4c-2026-11-17/native-model" cargo test --lib --features tiny-model structured_model_ -- --ignored --nocapture
```

### Project layout

| Path | What's there |
|---|---|
| `app/src-tauri/src/parse/` | Rule-based parser, one file per kind of command |
| `app/src-tauri/src/plan.rs` | Resolves names, expands filters, checks paths, builds the preview |
| `app/src-tauri/src/exec.rs` | Runs a confirmed plan and records how to undo each step |
| `app/src/` | The floating window: commands, settings, folder access |
| `website/` | The landing page, deployed to Vercel |
| `docs/readme/` | The banner at the top of this file, light and dark |
| `docs/icon/` | The app icon and the script that draws it |
| `BUILD.md` | The original build plan |

### Releases

Every pull request and every push to `main` that touches the app runs
[`.github/workflows/release.yml`](.github/workflows/release.yml). On pull
requests, the `quality` job checks version agreement, builds the frontend,
runs both Rust test suites, runs the rule-only cross-build equivalence suite in
both feature sets, and runs strict Clippy in both feature sets. On `main`, the
release job requires that gate, assigns one version to every manifest and
lockfile, builds a universal app for Apple silicon and Intel with
`--no-default-features`, signs it, verifies signatures, effective
entitlements and both CPU slices, and only then publishes:

- the `.dmg` for new installs,
- a signed update bundle and `latest.json` for in-app updates,
- release notes listing every change since the previous version.

Versions are `0.2.N`: each release is one past the newest published
`v0.2.N` tag, so pull request runs and failed runs never use up a number.
Change `SERIES` in the workflow to start a new line. The manifests in the
repository stay at `0.0.0`, so a local build is never mistaken for a release;
only the release job writes the real version.

Signing has two modes, chosen by which secrets exist:

- **Self-signed (current):** `VOXI_SIGNING_CERT` and
  `VOXI_SIGNING_CERT_PASSWORD` hold the "Voxi Self-Signed" certificate and its
  .p12 password. The app keeps one identity across versions, so macOS remembers
  its folder access, but it is not notarized and Gatekeeper asks once on first
  launch.
- **Developer ID:** also set `APPLE_SIGNING_IDENTITY` and `APPLE_TEAM_ID` (the
  signing identity and team), and `APPLE_API_KEY`, `APPLE_API_ISSUER` and
  `APPLE_API_KEY_P8` (notarization API credentials), and replace
  `VOXI_SIGNING_CERT` with the Developer ID certificate. The release is then
  notarized and stapled, and Gatekeeper acceptance is verified.

Setting only some of the Apple secrets fails the release. Both modes need
`TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (update
signing). Everyday local, rule-based use needs none of them.

### Website

`website/` is a static site with no build step. Its look comes from the app
icon: paper, ink, and the red smile that doubles as a sound wave, which ripples
in the hero while the demo command is typed. Serve it locally with
`python3 -m http.server -d website`. In Vercel, import this repo and set
**Root Directory** to `website`. Pushes that only touch the website or docs
don't build a new app.
