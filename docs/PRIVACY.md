# Privacy and Diagnostics

## Local-only operation

Voxi's parser is rule-based and runs locally on your Mac. There is no telemetry,
and nothing is uploaded.

## Dictation permissions

Voxi has no built-in audio recording or speech recognition and doesn't request
microphone or speech permissions. It works with external dictation apps such as
[Wispr Flow](https://wisprflow.ai) or [Hex by Anomaly](https://anoma.ly/hex/):
open Voxi, focus the command bar, dictate, then press Enter to review the
command.

(Note: Hex downloads a speech model once, then transcribes locally and pastes
into the focused app; its Mac download requires Apple silicon and macOS 15 or
later.)

## Local learning and feedback

Voxi learns from you without sending anything anywhere:

- **Teach a phrase.** If Voxi doesn't understand a sentence and you rephrase it
  within a few minutes, it offers to run the working command the next time you
  say the original. Taught phrases appear under Settings → Shortcuts.
- **Folder nicknames.** When a name matches several folders and you pick one,
  Voxi offers to remember that choice.
- **Wrong-result reports.** Mark a result as wrong and Voxi saves a readable
  JSON line on this Mac, with search counts (no file names). Show or clear them
  from Settings.

## Private operational diagnostics

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

Commands that fail to plan are logged as a code (for example a search limit, an
unreadable folder, or a timeout) with an entry count and a time bucket. Each
line contains only a stable error code, a stage, an operation ID, counts, and
(for storage failures) an I/O error kind and OS number.

No command text, folder names, filenames, paths, notes, or model text are ever
written, and nothing is uploaded. Command text and paths stay in local History,
which is a separate store with the 200-entry retention described in
[SAFETY.md](SAFETY.md).
