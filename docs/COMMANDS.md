# Commands and Behavior

This document covers the detailed rules behind searching, lists, quoting,
Finder actions, chaining, schedules, and shortcuts. For the full catalog of all
27 command types, see the [Commands page](https://tryvoxi.vercel.app/commands).

## How searching works

File commands search the folder you name and its subfolders, up to 12 levels
deep. This applies to groups of files and to individual file or folder names;
multiple matching names prompt you to choose. A folder name finds the nearest
match first, so “create a folder in dev named test” works even when `dev` holds
large projects; a same-named folder elsewhere at the same depth still asks.

Hidden folders, symlinked folders, `_to_delete`, macOS packages (apps, Xcode
projects, iWork and Photos libraries, and similar), and dependency or build
folders (`node_modules`, `Pods`, `DerivedData`, `__pycache__`, virtual
environments, and `target`, `build`, `dist`, or `vendor` next to their project
file) are skipped; previews that reach into subfolders name any they skipped.
Name one directly to work inside it.

Moves, copies, and deletes are the exception: they take only the files directly
in the folder you name, so files kept in its subfolders stay put. End the
command with “including subfolders” to reach them too, as in “delete PDFs in
Downloads including subfolders”, or list the files first and then say “delete
them”. Elsewhere in a command those words are refused rather than read as part
of a name; quote a name that contains them. Renames and image edits keep files
in their folders.

### Scan limits and progress

Searches reuse cached folder listings within a command. The window shows progress,
and you can cancel with Esc or Cancel. If a required scan reaches its depth, entry, or time
limit, or cannot read a folder, commands that change files stop with a message
naming the folder that caused it and how to fix it (name a smaller folder, or
exclude a subfolder with − in Settings → Folders); read-only results are marked
as partial. When a single name was found before the limit, Voxi asks you to
confirm it instead of failing.

Searches are limited to 50,000 entries per folder tree and 10 seconds; the
Spotlight fallback has a separate 2-second limit within that search deadline.

### Name matching

Spaces, underscores, and hyphens are equivalent in name filters. Individual
filename searches rank exact and normalized names before word matches and typo
suggestions. Loose or typo matches always ask you to choose, even if there is
only one suggestion; broad file filters never automatically expand to typos.

## Working with a list

After a list command, follow-up commands use that exact set of files. For
example, `list files in Downloads`, then `delete the last file` selects the last
displayed row; `move PDFs to Documents` selects only PDFs from that list.

Each displayed item is numbered: `delete 4` or `move 5 to Desktop` targets that
row. Spoken numbers such as “four” work too; quote a literal numeric filename.
Numbers stay attached to their original files until another list is displayed.
New files added after listing aren't included. Naming another source folder or
an unrelated file starts a fresh search.

The active scope is shown above the results; use **Clear scope** to reset it.
Closing Voxi or opening Settings also ends the scope. Changes still require a
preview and confirmation. If a listed file disappears or is replaced, run the
list command again. Positional references use the displayed rows (up to 200);
group commands can use every file in a complete list.

## Names, quotes, and sizes

Without a list, commands need a filename or a specific group. Quote literal
names containing command words, for example
`move "Notes to Self.txt" to Documents`. Quoted names also preserve words such
as “one”. Size filters require units; contradictory or repeated bounds are
refused rather than silently discarded.

Use `move these to Receipts in Voxi Demo` or `sort Voxi Demo in Downloads by type`
to name a folder and its parent. The parent must exist. A move or copy can
create the destination folder inside it. If the whole unquoted phrase also
names an existing folder, Voxi uses it or asks you to choose when both folders
exist. Quote the whole name to use it literally, as in
`sort "Things in Progress" by type`. Quote either part to make the parent
explicit, as in `move these to "Receipts by Month" in Downloads`, or use one
path such as `Downloads/Voxi Demo/Receipts`.

Use “undo last” for the newest change; selecting an earlier move or delete by
its operation type isn't supported.

## From Finder

Select files in Finder, right-click, and choose **Organize with Voxi** (under
Quick Actions or Services). To use **Open With**, choose **Other… → All
Applications** and select Voxi. You can also send files by dropping them onto
the Voxi app icon or with `open -a Voxi file1 file2`.
Voxi opens with those files as the ones “these” and “them” refer to, so you
can say “move these to Receipts”. Sending another selection replaces the
current list.

Folders, symbolic links, and anything outside the folders Voxi may use are left
out, and Voxi says which. The files are checked again before anything runs. If
the Service is missing, turn it on in System Settings → Keyboard → Keyboard
Shortcuts → Services (on macOS 12, System Preferences → Keyboard → Shortcuts →
Services).

## Chaining commands

Join up to eight folder-creation, move, copy, rename, or trash commands with
`and`, `then`, `and then`, or a semicolon. Voxi previews the complete chain,
asks for one confirmation, and records it as one change for **undo last**:

> create a folder in Downloads called PDFS and move PDFs into it

Here, `it` means the single folder created earlier in the chain. `that folder`
and `the new folder` also work, and `its contents`, `everything in it`, or
`the files in it` mean the files inside that folder, as in “create a folder in
Downloads named test1 and move its content to Pictures every week”. While the
folder is new it is empty, so the first run only creates it. If the move or copy
doesn't name a source, it uses the parent of the created folder, Downloads in
this example. An explicit source always wins. Multiple created folders require
an explicit destination. Quotes keep command words and separators inside names
literal.

Each step must be understood and plan successfully before a preview is offered.
Steps that reuse files from an earlier step must be run separately; creating a
folder and using it as a destination is supported. Clear any previous result
list first.

A time or repeat at the end schedules the whole chain, for example
`create a folder in Downloads called PDFS and move PDFs into it everyday`.
`everyday`, `every day`, and `daily` all mean once a day. Without an explicit
time, the first run is one day from now; add `at 9 pm` to choose a clock time.
Repeating chains plan again at each run, reusing existing folders and picking
up newly matching files. Read-only commands, Finder actions, and undo run
separately.

## Scheduled and repeating commands

Add a time, a delay, or a repeat to the end of a file command, review the
preview, then choose **Schedule task**:

- `move screenshots from Desktop to Screenshots folder at 9:40 pm`
- `move PDFs from Downloads to Documents after 3 hours`
- `clean up Downloads older than 90 days in 30 minutes`
- `delete dmg files from Downloads every 3 days`
- `sort Downloads by type every Monday at 8 am`

Clock times use your Mac's local timezone. A time that has passed means
tomorrow; you can also say `at 9:40 pm tomorrow` or use a 24-hour time such as
`at 21:40`. Delays accept whole hours or minutes, including spoken numbers
through ninety-nine, such as “three” or “twenty-one”.

Repeats accept `every 3 days`, `every other week`, `every Monday`,
`daily at 9 pm`, `hourly`, and `weekly`. A time of day works with days and
weeks; a weekday without a time runs at 9 am. Repeats are at least 15 minutes
apart.

- **One-time tasks** change only the files in their approved preview, excluding
  files added later. Changed or missing source files stop the task.
- **Repeating tasks** without a time of day run the approved preview immediately
  on confirmation, then repeat at the chosen interval; tasks with a time of day
  first run at the next occurrence of that time. Later runs plan the saved
  command again and act on whatever matches then, without a new preview. A
  repeating task can't use a previous list as its scope; name the files or
  folder instead.

Settings → Scheduled lists tasks, their results, and a cancellation button. A
run with nothing to do says “Nothing to change”. A run that can't plan, for
example because a folder is missing or a search couldn't finish, changes nothing
and says “Skipped:” with the reason.

Tasks survive restarting Voxi. Keep Voxi running for on-time execution; if it is
closed or your Mac is asleep, an overdue task runs once when Voxi next runs or
your Mac wakes. No task wakes the Mac or launches Voxi. Undo, read-only, and
Finder commands cannot be scheduled.

For how interrupted tasks, destination collisions, and schedule storage errors
are handled, see [SAFETY.md](SAFETY.md).

## Shortcuts

Save a phrase and a list of commands in Settings → Shortcuts. Saying the phrase
runs them all as one preview with one undo. "Run my usual" and "run my usuals"
both work.

A shortcut can also hold a single read-only command, like "list downloads",
which runs as if you typed it; read-only commands can't be mixed with commands
that change files. A command in a shortcut that has nothing to do is skipped and
the preview says so (“1 of 2 commands”); any other failure stops the whole
shortcut before anything changes.

Shortcut size and command count limits are listed in [SAFETY.md](SAFETY.md#input-bounds).
