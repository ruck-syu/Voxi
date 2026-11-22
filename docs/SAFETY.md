# Safety, Permissions, and Recovery

## Core guarantees

- **Nothing changes until you press Enter.** Every command is previewed first.
- **Nothing is erased.** "Delete" and "clean up" move files to the Trash.
- **Nothing is overwritten.** A clashing name becomes `report 2.pdf`, and the preview says so. Destinations are claimed atomically, so a file created after the preview cannot be clobbered.
- **Everything can be undone.** "Undo last" reverses the newest change as one step, after its own preview. If a chain or shortcut fails partway, the steps that completed stay undoable when their history entry was saved, and Voxi reports the failure.
- **Voxi stays in its lane.** It only touches Desktop, Downloads, Documents, Pictures, and folders you add in Settings. A folder that is a symlink (say, Downloads on an external drive) works under either path, but other links pointing at it are refused, and if it moves or is remounted Voxi stops and asks you to restart or update Settings → Folders.
- **No guessing.** When a name matches several files, Voxi lists them and asks which one; say a number to answer.

## Input bounds

A command may be at most 16 KiB of UTF-8 text. A saved shortcut holds at most 8
commands whose combined text is at most 16 KiB. Larger input is refused before
Voxi plans anything.

## Permissions

On first launch, Voxi opens a welcome page to request standard macOS **Files and
Folders** grants for Desktop, Downloads, Documents, and Pictures. That is all
Voxi needs by default — it does not need Full Disk Access by default.

If you ask Voxi to work in a protected location outside those folders — such as
another app's container — and the folder grant is not enough, enable **Full Disk
Access** for Voxi under System Settings → Privacy & Security, and only for those
locations.

If a grant is missing later, the folder-access page offers **Check again** and
**Open System Settings**; a failed request offers **Retry**.

## Operation journal, crash recovery, and History

Every file operation is written to a journal before it runs. If Voxi is killed
partway through:

- Steps it had already recorded as complete are recovered into History.
- The interrupted step is **never replayed**, and its result must be checked by
  hand.
- If Voxi stops before the history entry was saved, that step has no durable
  undo.

For scheduled tasks:

- Destination collisions never overwrite files, and every completed run enters
  History so `undo last` can reverse it.
- An interrupted **one-time** task is marked failed.
- An interrupted **repeating** task skips the missed run and schedules its next
  one.

In either case, Settings → Scheduled shows a recovery notice listing the files
to check, with **Mark as reviewed**. Reviewing archives the journal evidence and
keeps any recovered undo, but does not retry the operation.

History keeps the 200 most recent changes, and `undo last` reaches the newest
one only. Selecting an earlier move or delete by its operation type isn't
supported.

## Corrupt settings, history, or schedules

If settings, history, or schedules are corrupt, Voxi preserves a
`.corrupt-<timestamp>` backup, shows a notice, and starts with defaults; later
saves proceed normally, with the original kept in the backup.

Saves are blocked — and the original left untouched — only when:

- the backup could not be written,
- the file could not be read, or
- it declares a newer version from a later Voxi.

If saved schedules cannot be read, Voxi preserves a backup and shows a recovery
notice in Settings → Scheduled. A storage error disables scheduling for that
session while the rest of the app remains available.

This recovery evidence is kept separate from the private operational diagnostics
described in [PRIVACY.md](PRIVACY.md).
