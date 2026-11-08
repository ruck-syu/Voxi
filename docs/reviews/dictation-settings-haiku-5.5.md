> Historical review: built-in Apple dictation has since been removed. These findings describe the earlier implementation.

# Opt-in dictation settings — Haiku 5.5 max review

Claude Haiku 5.5, effort `max`, performed a read-only review of the settings,
command-bar pill and interactions with the existing Apple speech integration.
It reported no high-severity findings. Findings were supplied to the parent
agent; the fixes below were made afterwards and were not re-reviewed.

## Fixes after review

| Finding | Change |
|---|---|
| Successful dictation clears typed draft | Removed unconditional input clearing. Final dictation appends to a non-empty command draft and leaves the combined text for editing; Enter previews it. Cancellation preserves the draft. Added a draft-composition regression test. |
| Visible pill label does not match accessible name | Removed the changing `aria-label` and toggle semantics. The visible action text is the accessible name. The action remains focusable while temporarily unavailable and its handler honors `aria-disabled`. |
| State announcements not reliable | Kept a permanently mounted polite, atomic status region; it announces starting/listening/finalizing states and transcription/status text. |
| Disable guarantee not tested | Extracted the actual locked start/commit helpers used by the native command and settings save. Tests verify disabled capture never schedules work, disabled commits cancel, and an in-flight start is serialized before cancellation. |
| Esc cancels and closes the window | Esc returns immediately after cancelling an active recording. |
| Disabled shortcut still consumes key | Moved the feature flag into the keyboard shortcut condition. |
| Overlapping saves can revert opt-in | Added a synchronous save gate, disabled settings fields while a save is pending, and blocked tab switches/Done/Esc until completion. Keyboard focus is restored if disabling the field temporarily dropped it. |
| Hex launcher blocks UI thread | Marked the fixed-URL command for asynchronous execution. |
| Misleading microphone/Hex copy | Changed to Apple's speech recognition, explained the one-time Hex model download, and described pasting into the focused text field. Recommendation does not claim measured accuracy superiority or an automatic Voxi integration. |
| README names removed Cancel control | Documented Esc cancellation and the current pill/Enter behavior. |
| Heading/error semantics | Recommendation uses an h2 under Settings; save errors have alert semantics. |
| Shared header CSS and pill wrapping | Scoped the new flex-header rule to the command header, preserving the Settings header gap. Added no-wrap to the pill. |

## Checks

- `cargo test --lib`: 125 passed, one held-out test ignored, including opt-in migration/persistence and disable/concurrency tests.
- Voice intent/session JS tests: eight passed.
- TypeScript/Vite production build passed; local debug application rebuilt.
- Earlier native AX smoke check verified the default command bar has no dictation control, and Settings → Dictation exposes the toggle, accuracy note, Hex button and platform requirements.
- No microphone recording or VoiceOver session was performed as part of this settings change.

## Remaining limits and intentional behavior

The generic “Type or dictate” placeholder includes external tools such as Hex
and Wispr Flow and therefore remains visible when Apple's opt-in is off. The
pill follows Voxi's existing light/dark tokens; it is a light pill in dark mode,
matching the supplied reference, and uses the inverse palette in light mode.

The reviewer also raised runtime-dependent or theoretical risks in the existing
speech engine (permission-prompt focus, wall-clock session IDs across reloads,
cancel-all ordering and rare native string failures). These were not reproduced
or changed in this settings patch. Live microphone, VoiceOver/Voice Control,
device-switch and permission-flow checks remain release verification work.

Hex references checked during implementation:

- [Official download page](https://anoma.ly/hex/)
- [Official repository and setup behavior](https://github.com/anomalyco/hex)
- [License](https://github.com/anomalyco/hex/blob/dev/LICENSE)
