import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  buildReleaseNotes,
  buildUpdaterNotes,
  bumpVersion,
  classifyCommit,
  computeReleasePlan,
  extractCommitInfo,
  latestSemverTag,
  parseSemver,
} from './next-version.mjs';

const SCRIPT_PATH = fileURLToPath(new URL('./next-version.mjs', import.meta.url));

function makeRepo(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'voxi-next-version-'));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  const git = (...args) =>
    execFileSync('git', args, { cwd: dir, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });

  git('init', '-b', 'main');
  git('config', 'user.name', 'Voxi Test');
  git('config', 'user.email', 'test@voxi.dev');

  const writeAndCommit = (files, message) => {
    for (const [relPath, content] of Object.entries(files)) {
      const fullPath = path.join(dir, relPath);
      fs.mkdirSync(path.dirname(fullPath), { recursive: true });
      fs.writeFileSync(fullPath, content, 'utf8');
    }
    git('add', '-A');
    git('commit', '-m', message);
  };

  const mergePr = ({ branch, prNumber, prTitle, prBody = '', commits }) => {
    git('checkout', '-b', branch);
    for (const { files, message } of commits) {
      writeAndCommit(files, message);
    }
    git('checkout', 'main');
    const mergeMessage = [
      `Merge pull request #${prNumber} from ruck-syu/${branch}`,
      '',
      prTitle,
      ...(prBody ? ['', prBody] : []),
    ].join('\n');
    git('merge', '--no-ff', branch, '-m', mergeMessage);
  };

  const tag = (name) => git('tag', name);

  return { dir, git, writeAndCommit, mergePr, tag };
}

test('semver parsing, numeric sorting, and strictly increasing bumps', () => {
  assert.equal(parseSemver('v0.0.3')?.version, '0.0.3');
  assert.equal(parseSemver('v0.01.0'), null);
  assert.equal(parseSemver('v1.2.3-rc.1'), null);
  assert.equal(parseSemver('release-1'), null);

  const latest = latestSemverTag(['v0.0.2', 'v0.0.10', 'v0.0.3', 'v0.0.11-beta.1', 'notes']);
  assert.equal(latest?.tag, 'v0.0.10');

  assert.equal(bumpVersion(null, 'patch'), '0.0.1');
  assert.equal(bumpVersion(null, 'minor'), '0.1.0');
  assert.equal(bumpVersion(null, 'major'), '1.0.0');
  assert.equal(bumpVersion(latest, 'patch'), '0.0.11');
  assert.equal(bumpVersion(latest, 'minor'), '0.1.0');
  assert.equal(bumpVersion(latest, 'major'), '1.0.0');

  const minorTag = latestSemverTag(['v0.0.10', 'v0.1.0', 'v0.0.99']);
  assert.equal(minorTag?.tag, 'v0.1.0');
  assert.equal(bumpVersion(minorTag, 'patch'), '0.1.1');
});

test('commit classification handles PR merges, conventional prefixes, breaking changes, and skip', () => {
  const prMerge = extractCommitInfo({
    parents: ['aaa', 'bbb'],
    subject: 'Merge pull request #24 from ruck-syu/feat/reminders',
    body: 'feat(scheduler): add recurring reminders\n\nExtra context',
  });
  assert.equal(prMerge.prNumber, 24);
  assert.equal(prMerge.title, 'feat(scheduler): add recurring reminders');
  assert.deepEqual(classifyCommit(prMerge), { bump: 'minor', skip: false });

  assert.deepEqual(classifyCommit({ title: 'Fix Finder handoff', body: '' }), {
    bump: 'patch',
    skip: false,
  });
  assert.deepEqual(classifyCommit({ title: 'feat!: new command grammar', body: '' }), {
    bump: 'major',
    skip: false,
  });
  assert.deepEqual(classifyCommit({ title: 'fix(parser)!: reject ambiguous quotes', body: '' }), {
    bump: 'major',
    skip: false,
  });
  assert.deepEqual(
    classifyCommit({ title: 'Refactor storage', body: 'Details\n\nBREAKING CHANGE: resets state' }),
    { bump: 'major', skip: false },
  );
  assert.deepEqual(
    classifyCommit({ title: 'ci: update release script [skip release]', body: '' }),
    { bump: 'patch', skip: true },
  );
});

test('skips release when only non-app files changed since last tag', (t) => {
  const repo = makeRepo(t);
  repo.writeAndCommit({ 'app/src/main.ts': 'v1\n', 'README.md': 'v1\n' }, 'Initial app');
  repo.tag('v0.0.3');
  repo.writeAndCommit({ '.github/workflows/release.yml': 'updated\n' }, 'Tweak workflow');

  const plan = computeReleasePlan({ repoDir: repo.dir });
  assert.equal(plan.shouldRelease, false);
  assert.equal(plan.previousTag, 'v0.0.3');
  assert.match(plan.reason, /No changes in app\/ or crates\//);
});

test('releases patch by default for plain-English PR merges touching app/ or crates/', (t) => {
  const repo = makeRepo(t);
  repo.writeAndCommit({ 'app/src/main.ts': 'v1\n', 'crates/core/lib.rs': 'v1\n' }, 'Initial app');
  repo.tag('v0.0.3');

  repo.mergePr({
    branch: 'fix/demo-findings',
    prNumber: 20,
    prTitle: 'Fix Finder handoff and demo bugs',
    commits: [
      { files: { 'crates/core/lib.rs': 'v2\n' }, message: 'feat: internal wip commit' },
      { files: { 'app/src/main.ts': 'v2\n' }, message: 'Polish error copy' },
    ],
  });

  const plan = computeReleasePlan({ repoDir: repo.dir, repository: 'ruck-syu/Voxi' });
  assert.equal(plan.shouldRelease, true);
  assert.equal(plan.bump, 'patch');
  assert.equal(plan.version, '0.0.4');
  assert.equal(plan.tag, 'v0.0.4');
  assert.deepEqual(plan.bullets, ['Fix Finder handoff and demo bugs (#20)']);
  assert.equal(plan.updaterNotes, "## What's new\n- Fix Finder handoff and demo bugs (#20)");
  assert.match(plan.releaseNotes, /## What's new\n- Fix Finder handoff and demo bugs \(#20\)/);
  assert.match(plan.releaseNotes, /Voxi_0\.0\.4_universal\.dmg/);
  assert.match(plan.releaseNotes, /compare\/v0\.0\.3\.\.\.v0\.0\.4/);
});

test('bumps minor for feat:, minor for breaking changes before 1.0, major after', (t) => {
  const repo = makeRepo(t);
  repo.writeAndCommit({ 'app/src/main.ts': 'v1\n' }, 'Initial app');
  repo.tag('v0.0.3');

  repo.mergePr({
    branch: 'feat/schedules',
    prNumber: 25,
    prTitle: 'feat: add recurring schedules',
    commits: [{ files: { 'app/src/main.ts': 'v2\n' }, message: 'Add scheduler loop' }],
  });

  const minorPlan = computeReleasePlan({ repoDir: repo.dir });
  assert.equal(minorPlan.shouldRelease, true);
  assert.equal(minorPlan.bump, 'minor');
  assert.equal(minorPlan.version, '0.1.0');

  repo.tag('v0.1.0');
  repo.mergePr({
    branch: 'feat/v1',
    prNumber: 26,
    prTitle: 'feat!: stabilize Voxi 1.0 command grammar',
    commits: [{ files: { 'app/src/main.ts': 'v3\n' }, message: 'Finalize grammar' }],
  });

  const breakingPlan = computeReleasePlan({ repoDir: repo.dir });
  assert.equal(breakingPlan.shouldRelease, true);
  assert.equal(breakingPlan.bump, 'minor');
  assert.equal(breakingPlan.version, '0.2.0');

  const manualMajor = computeReleasePlan({ repoDir: repo.dir, event: 'workflow_dispatch', bump: 'major' });
  assert.equal(manualMajor.version, '1.0.0');

  repo.tag('v1.0.0');
  repo.mergePr({
    branch: 'feat/v2',
    prNumber: 27,
    prTitle: 'feat!: replace the command grammar',
    commits: [{ files: { 'app/src/main.ts': 'v4\n' }, message: 'New grammar' }],
  });
  const majorPlan = computeReleasePlan({ repoDir: repo.dir });
  assert.equal(majorPlan.bump, 'major');
  assert.equal(majorPlan.version, '2.0.0');
});

test('honors [skip release] without blocking subsequent app PRs', (t) => {
  const repo = makeRepo(t);
  repo.writeAndCommit({ 'app/src/main.ts': 'v1\n' }, 'Initial app');
  repo.tag('v0.0.3');

  repo.mergePr({
    branch: 'ci/release-script',
    prNumber: 27,
    prTitle: 'ci: add release script [skip release]',
    commits: [{ files: { 'app/scripts/helper.mjs': 'export {};\n' }, message: 'Add helper' }],
  });

  const skipped = computeReleasePlan({ repoDir: repo.dir });
  assert.equal(skipped.shouldRelease, false);
  assert.match(skipped.reason, /\[skip release\]/);

  // A follow-up workflow-only PR should still not trigger a release for the skipped app PR.
  repo.mergePr({
    branch: 'ci/tweak-yaml',
    prNumber: 28,
    prTitle: 'Adjust workflow timeout',
    commits: [{ files: { '.github/workflows/release.yml': 'v2\n' }, message: 'Timeout' }],
  });
  const stillSkipped = computeReleasePlan({ repoDir: repo.dir });
  assert.equal(stillSkipped.shouldRelease, false);

  // A real app PR after that releases normally.
  repo.mergePr({
    branch: 'fix/bug',
    prNumber: 29,
    prTitle: 'Fix undo preview',
    commits: [{ files: { 'app/src/main.ts': 'v2\n' }, message: 'Fix undo' }],
  });
  const released = computeReleasePlan({ repoDir: repo.dir });
  assert.equal(released.shouldRelease, true);
  assert.equal(released.version, '0.0.4');
  assert.equal(released.bullets[0], 'Fix undo preview (#29)');
  // [skip release] token is stripped from notes if a skipped PR rides along.
  assert.ok(released.bullets.every((b) => !/\[skip release\]/i.test(b)));
});

test('workflow_dispatch forces a release with optional bump override and fallback notes', (t) => {
  const repo = makeRepo(t);
  repo.writeAndCommit({ 'app/src/main.ts': 'v1\n' }, 'Initial app');
  repo.tag('v0.0.3');

  const autoPlan = computeReleasePlan({
    repoDir: repo.dir,
    event: 'workflow_dispatch',
    bump: '',
  });
  assert.equal(autoPlan.shouldRelease, true);
  assert.equal(autoPlan.bump, 'patch');
  assert.equal(autoPlan.version, '0.0.4');
  assert.equal(autoPlan.updaterNotes, "## What's new\n- Maintenance release");

  const minorPlan = computeReleasePlan({
    repoDir: repo.dir,
    event: 'workflow_dispatch',
    bump: 'minor',
  });
  assert.equal(minorPlan.shouldRelease, true);
  assert.equal(minorPlan.bump, 'minor');
  assert.equal(minorPlan.version, '0.1.0');
});

test('initial release works when no semver tags exist yet', (t) => {
  const repo = makeRepo(t);
  repo.writeAndCommit({ 'app/src/main.ts': 'v1\n' }, 'Initial Voxi app');

  const plan = computeReleasePlan({ repoDir: repo.dir });
  assert.equal(plan.shouldRelease, true);
  assert.equal(plan.previousTag, null);
  assert.equal(plan.version, '0.0.1');
  assert.deepEqual(plan.bullets, ['Initial Voxi app']);
});

test('CLI writes github outputs and note files, and enforces --require-release', (t) => {
  const repo = makeRepo(t);
  repo.writeAndCommit({ 'app/src/main.ts': 'v1\n' }, 'Initial app');
  repo.tag('v0.0.3');
  repo.writeAndCommit({ 'app/src/main.ts': 'v2\n' }, 'feat: support quick actions');

  const outFile = path.join(repo.dir, 'github-output.txt');
  const notesFile = path.join(repo.dir, 'release-notes.md');
  const updaterFile = path.join(repo.dir, 'updater-notes.md');

  execFileSync(
    process.execPath,
    [
      SCRIPT_PATH,
      '--repo',
      repo.dir,
      '--require-release',
      '--github-output',
      outFile,
      '--write-notes',
      notesFile,
      '--write-updater-notes',
      updaterFile,
      '--notarized',
      'false',
    ],
    { encoding: 'utf8' },
  );

  const ghOut = fs.readFileSync(outFile, 'utf8');
  assert.match(ghOut, /^should_release=true$/m);
  assert.match(ghOut, /^version=0\.1\.0$/m);
  assert.match(ghOut, /^bump=minor$/m);
  assert.match(ghOut, /^previous_tag=v0\.0\.3$/m);

  const updaterNotes = fs.readFileSync(updaterFile, 'utf8');
  assert.equal(updaterNotes, "## What's new\n- feat: support quick actions\n");

  const releaseNotes = fs.readFileSync(notesFile, 'utf8');
  assert.match(releaseNotes, /## What's new\n- feat: support quick actions/);
  assert.match(releaseNotes, /not notarized yet/);

  // Tagging HEAD means no app changes remain, so --require-release must fail.
  repo.tag('v0.1.0');
  assert.throws(
    () =>
      execFileSync(process.execPath, [SCRIPT_PATH, '--repo', repo.dir, '--require-release'], {
        stdio: 'pipe',
      }),
    /Release required, but skipped/,
  );
});

test('updater notes match app/src/updates.ts whatsNew format', () => {
  const notes = buildUpdaterNotes(['Fix Finder handoff (#20)', 'feat: add schedules (#21)']);
  const section = notes.split(/^## /m).find((s) => /^what's new/i.test(s)) ?? notes;
  const bullets = section
    .split('\n')
    .filter((l) => l.startsWith('- '))
    .map((l) => l.slice(2).trim());
  assert.deepEqual(bullets, ['Fix Finder handoff (#20)', 'feat: add schedules (#21)']);

  const full = buildReleaseNotes({ version: '0.1.0', bullets: ['Fix Finder handoff (#20)'] });
  assert.ok(full.startsWith("## What's new\n- Fix Finder handoff (#20)\n"));
});
