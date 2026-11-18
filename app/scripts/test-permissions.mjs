#!/usr/bin/env node
// Static checks for the permission-recovery UI (T6 / F03 / U02 / F05).
// Verifies the error/Retry wiring and least-privilege guidance are present in
// the shipped source, and that the browser mocks can exercise each path.
//
// NOTE: the interactive VoiceOver / keyboard walkthrough at 200% zoom with
// reduced motion (onboarding, ambiguity, confirm/undo, settings, errors) is
// still a MANUAL check. This script does not replace it.
//
// No third-party dependencies: Node built-ins only. Run from anywhere with
// `node scripts/test-permissions.mjs` (or from the repo root with the app/
// prefix). When dist/assets exists, user-facing strings are also checked
// against the built bundle.

import assert from 'node:assert/strict';
import { runInNewContext } from 'node:vm';
import { existsSync, readFileSync, readdirSync } from 'node:fs';

const root = new URL('../', import.meta.url);
const read = (path) => readFileSync(new URL(path, root), 'utf8');

const app = read('src/App.tsx');
const api = read('src/api.ts');
const mock = read('src/mock.ts');
const css = read('src/styles.css');

const checks = [];
const check = (label, ok) => checks.push([label, Boolean(ok)]);

// F03: a rejected folder-access request is caught and shown with a Retry.
check('request() catches a rejected requestFolderAccess', /requestFolderAccess\(\)[\s\S]{0,160}catch \(e\)/.test(app));
check('request() sets a readable error state', /setRequestError\(/.test(app));
check('Retry button is rendered and calls request()', /"Retry"/.test(app) && /onClick=\{request\}/.test(app));
check('error regions use role="alert" and aria-live', /role="alert" aria-live="assertive"/.test(app));
check('focus moves to the recovery control on failure', /actionRef\.current\?\.focus\(\)/.test(app) && /settingsErrorRef\.current\?\.focus\(\)/.test(app));

// F03: the open-settings promise is awaited and its failure is surfaced.
check('openPrivacySettings is awaited', /await api\.openPrivacySettings\(\)/.test(app));
check('open-settings failure sets an error state', /openPrivacySettings\(\)[\s\S]{0,160}catch \(e\)/.test(app));

// U02: per-folder grant is the default; FDA is explained as scoped recovery.
check('Files and Folders path is shown', /Files and Folders/.test(app));
check('Privacy & Security path is shown', /Privacy &amp; Security/.test(app));
check('Full Disk Access is explained', /Full Disk Access/.test(app));
check('FDA is not required by default', /does not need Full Disk Access by default/.test(app));
check('FDA scoped to locations outside Desktop/Documents/Downloads', /outside Desktop, Documents or Downloads/.test(app));

// Shared error formatting and switchable browser scenarios.
check('errorMessage helper is exported', /export function errorMessage/.test(api));
for (const scenario of ['permission-reject', 'permission-denied', 'permission-flaky', 'settings-fail']) {
  check(`mock scenario ${scenario}`, mock.includes(scenario));
}

// Accessibility styling for the focused error message.
check('focus style for error message', /\.access-error \.hint:focus-visible/.test(css));

// T5: the UI mirrors parse::MAX_COMMAND_BYTES and check_shortcut_commands.
check('MAX_COMMAND_BYTES is 16,384 in the UI', /MAX_COMMAND_BYTES = 16_384/.test(app));
check('command box measures UTF-8 bytes', /new TextEncoder\(\)\.encode\(text\)\.length/.test(app));
check('exact backend refusal message is used', app.includes('That command is too long.'));
check('oversized command is never submitted', /if \(tooLong\) return/.test(app) && /utf8Bytes\(command\) > MAX_COMMAND_BYTES/.test(app));
check('command message is an alert linked by aria-describedby', /aria-describedby="command-length-error"/.test(app) && /id="command-length-error" role="alert"/.test(app));
check('shortcut expansions are capped at eight commands', /MAX_SHORTCUT_COMMANDS = 8/.test(app) && /s\.commands\.length > MAX_SHORTCUT_COMMANDS/.test(app));
check('shortcut expansion bytes are summed against the cap', /total > MAX_COMMAND_BYTES/.test(app));
check('shortcut save is disabled while over the cap', /disabled=\{!canSave\}/.test(app) && /next\.some\(\(s\) => shortcutLengthIssue\(s\)\.any\)/.test(app));
// Remove is the way out when the edited shortcut is itself over the cap, so it
// uses a separate canRemove (list minus this shortcut) rather than canSave.
check('Remove uses a separate canRemove flag, not canSave', /className="link remove" disabled=\{!canRemove\}/.test(app) && /canRemove\??: boolean/.test(app));
check('canRemove validates the list without this shortcut', /list\.filter\(\(_, j\) => j !== i\)\.some\(\(x\) => shortcutLengthIssue\(x\)\.any\)/.test(app));
check('shortcut message is an inline alert', /className="shortcut-error hint warn"/.test(app) && /role="alert"/.test(app));
check('empty length messages are hidden, not truncated text', /\.command-error:empty/.test(css) && /\.shortcut-error:empty/.test(css));
check('mock refuses oversized commands', /utf8Bytes\(text\) > MAX_COMMAND_BYTES/.test(mock));
check('mock refuses oversized shortcut expansions', /s\.commands\.length > 8/.test(mock) && /total > MAX_COMMAND_BYTES/.test(mock));

// Dismissal regression: native clicks inside the live AppKit frame are ignored.
const outsideClick = read('src-tauri/src/outside_click.rs');
check('outside-click monitor hit-tests AppKit screen coordinates before emitting',
  /ns_window\(\)/.test(outsideClick) && /ns\.frame\(\)/.test(outsideClick)
  && /NSEvent::mouseLocation\(\)/.test(outsideClick)
  && outsideClick.includes('if point_in_frame(')
  && outsideClick.indexOf('if point_in_frame(') < outsideClick.indexOf('window.emit('));

// Execute the actual blur effect against a deterministic clock and focus state.
const blurEffect = app.match(/useEffect\(\(\) => \{\n    if \(!inTauri\) return;\n    const onBlur = \(\) => \{([\s\S]*?)\n  \}, \[close, cancelBlurClose\]\);/);
check('blur dismissal has a deferred, cancellable effect', blurEffect);
if (blurEffect) {
  const listeners = new Map();
  const timers = new Map();
  let nextTimer = 0, closes = 0, focused = false, now = 1000, lastDelay = 0;
  const blurTimer = { current: null }, busyRef = { current: false };
  const shownAt = { current: 0 }, showId = { current: 1 }, awaitingShow = { current: null };
  const cancelSource = app.match(/const cancelBlurClose = useCallback\(\(\) => \{([\s\S]*?)\n  \}, \[\]\);/);
  assert.ok(cancelSource, 'blur cancellation callback exists');
  const cancelBlurClose = runInNewContext(`(() => {${cancelSource[1]} })`, {
    blurTimer, clearTimeout: (id) => timers.delete(id),
  });
  const cleanup = runInNewContext(`(() => { const onBlur = () => {${blurEffect[1]} })()`, {
    blurTimer, busyRef, shownAt, showId, awaitingShow, cancelBlurClose,
    close: () => { closes++; },
    document: { hasFocus: () => focused }, performance: { now: () => now },
    setTimeout: (fn, delay) => { assert.ok(delay >= 150); lastDelay = delay; timers.set(++nextTimer, fn); return nextTimer; },
    window: {
      addEventListener: (name, fn) => listeners.set(name, fn),
      removeEventListener: (name) => listeners.delete(name),
    },
  });
  const blur = () => listeners.get('blur')();
  const tick = () => { const pending = [...timers.values()]; timers.clear(); pending.forEach((fn) => fn()); };
  blur(); assert.equal(closes, 0);
  listeners.get('focus')(); tick(); assert.equal(closes, 0);
  blur(); focused = true; tick(); assert.equal(closes, 0);
  focused = false; blur(); tick(); assert.equal(closes, 1);
  busyRef.current = true; blur(); assert.equal(timers.size, 0);
  busyRef.current = false; blur(); busyRef.current = true; tick(); assert.equal(closes, 1);
  busyRef.current = false; now = 1000; blur(); assert.equal(lastDelay, 150);
  // A blur inside the opening grace waits for the grace to end, then closes.
  now = 100; blur(); assert.equal(lastDelay, 350); tick(); assert.equal(closes, 1);
  blur(); now = 1000; tick(); assert.equal(closes, 2);
  blur(); showId.current++; tick(); assert.equal(closes, 2);
  blur(); awaitingShow.current = 2; tick(); assert.equal(closes, 2);
  awaitingShow.current = null; blur(); cleanup(); tick(); assert.equal(closes, 2);
  assert.equal(listeners.size, 0);
  check('blur waits, cancels on focus, rechecks guards, defers through the opening grace, and closes on a genuine switch', true);
}

// If a production bundle exists, confirm the user-facing strings ship too.
const distAssets = new URL('dist/assets/', root);
if (existsSync(distAssets)) {
  const js = readdirSync(distAssets)
    .filter((file) => file.endsWith('.js'))
    .map((file) => readFileSync(new URL(file, distAssets), 'utf8'))
    .join('\n');
  if (js) {
    for (const text of ['Full Disk Access', 'Files and Folders', 'Retry', 'check folder access', 'open System Settings', 'That command is too long.']) {
      check(`bundle contains "${text}"`, js.includes(text));
    }
  }
} else {
  console.log('Note: no dist/assets bundle yet (source-only checks). Run `npm run build` to also verify shipped strings.');
}

let failed = 0;
for (const [label, ok] of checks) {
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${label}`);
  if (!ok) failed += 1;
}
assert.equal(failed, 0, `${failed} permission check(s) failed`);
console.log(`Permissions checks passed (${checks.length} checks). Manual VoiceOver/keyboard walkthrough still required.`);
