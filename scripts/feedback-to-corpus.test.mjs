import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

test('triage retains planning failures across duplicate reports', (t) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'voxi-triage-'));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  const input = path.join(dir, 'reports.jsonl');
  const output = path.join(dir, 'triage.txt');
  fs.writeFileSync(input, [
    { command: 'move PDFs to Archive', intent: { intent: 'move' }, outcome: { status: 'problem' } },
    { command: 'Move PDFs to Archive', intent: { intent: 'move' }, outcome: { status: 'plan' } },
    { command: 'make a sandwich', intent: null, outcome: { status: 'not_understood' } },
    { command: 'delete installers', intent: { intent: 'trash' }, outcome: { status: 'done', error: 'execution error' } },
  ].map((report) => JSON.stringify(report)).join('\n'));

  const script = fileURLToPath(new URL('./feedback-to-corpus.mjs', import.meta.url));
  execFileSync(process.execPath, [script, input, output]);
  const triage = fs.readFileSync(output, 'utf8');
  assert.match(triage, /outcome=plan[\s\S]+# planner regression:.*src\/plan\/messy_tests\.rs\.[\s\S]+Move PDFs to Archive => \?/);
  assert.equal(triage.match(/# planner regression:/g)?.length, 1);
  assert.match(triage, /# parsed: refused\nmake a sandwich => \?/);
});
