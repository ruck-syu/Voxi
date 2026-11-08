> Historical review: built-in Apple dictation has since been removed. These findings describe the earlier implementation.

# Apple voice input — Opus 5.5 review and fixes

Review completed with Claude Opus 5.5. The reviewer inspected the diff and
untracked voice files without modifying code or running tests. Its original
findings included both code-proven issues and runtime-dependent risks; the
changes below were made afterwards by the implementation agent, not re-reviewed.

## Findings and disposition

| Finding | Resolution |
|---|---|
| Hardened-runtime microphone entitlement missing | Added `Entitlements.plist` with `com.apple.security.device.audio-input`, referenced by Tauri's macOS bundle configuration. Verified the entitlement on a locally signed native test executable. |
| Blur leaves recording active | Only permission setup suppresses normal blur handling. The voice controller cancels on blur after setup; a native AppKit observer also cancels when the application resigns active. |
| Enter can submit interim text | Interim speech now appears in a separate display and never enters the editable command box. Enter during a recording finalizes capture and does not submit the draft. Cancel/error clears the interim display and preserves the typed draft. |
| On-device support does not guarantee installed/usable assets | Failed on-device sessions explain how to enable Dictation and download language assets. No automatic online retry is performed. Online recognition is selected only when on-device recognition is unsupported, with disclosure. |
| Audio input changes leave a stalled session | Added an AVAudioEngine configuration observer that stops the session with an actionable error. Observers are removed on completion. |
| Stop-before-start race | Native stopped-ID watermark and latest-start ID reject obsolete starts. Second toggle during asynchronous listener registration cancels registration instead of sending a premature finalization. |
| Busy state survives CommandView unmount | Added cleanup resetting the parent's busy flag. |
| Native compiler flags ignore deployment environment | Read `MACOSX_DEPLOYMENT_TARGET` (default 12.0), track it for rebuilds, add ARC exception cleanup and profile-appropriate optimization/debug symbols. |
| Short finalization timeout | Increased final-result allowance from 3 to 8 seconds; interim text is never promoted to a final result. |
| Session time limit can submit background speech | At 60 seconds, cancel with an explanation instead of finalizing/submitting. |
| Page reload can leave capture running | Native Tauri page-load handler cancels the last session, independently of JS unload delivery. |
| Stale status and English-only control words | Terminal events clear status; README documents the English `confirm`/`cancel` words and current English command grammar. |
| Insufficient state-machine tests | Extracted a testable voice-session controller and added tests for partial/final separation, finalization, cancellation, late events, context changes, pending listener registration and errors. |

## Checks performed after the fixes

- `npm run build`: passed (TypeScript and Vite).
- `node --experimental-strip-types --test src/voice-intent.test.mjs src/voice-session.test.mjs`: seven tests passed.
- Native Objective-C bridge compiled for arm64 and x86_64, targeting macOS 12.
- Full Tauri Rust library metadata compilation passed with its coherent cached dependency set. A normal Cargo release/package build was not run.
- Permission-free native lifecycle executable passed: completion fires once, and stopped/obsolete IDs do not start permission or microphone work.
- Native test executable signed with hardened runtime and the new entitlement. `codesign -d --entitlements -` confirmed audio-input access. This verifies entitlement/signing syntax, not microphone permission behavior in the packaged Voxi application.
- Voice JS and native lifecycle tests added to the release workflow.

## Remaining runtime verification

No live microphone recording was performed. Before release, test the signed
universal application: first-use prompts, denial and re-grant, a real transcript,
Dictation disabled/missing language assets, unplugging/changing microphones,
focus loss, reload, finalizing with Enter and spoken choice/confirmation.
The installed app has not been replaced.

This implementation uses Apple's `SFSpeechRecognizer` on the existing macOS 12+
baseline. The newer `SpeechAnalyzer` engine remains a separate follow-up.
