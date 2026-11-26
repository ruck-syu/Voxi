#!/usr/bin/env node
// Single source of truth for Voxi release gating, SemVer bumping, and release notes.
// Works offline against local git history with zero third-party dependencies (Node 22+).

import { execFileSync } from 'node:child_process';
import { appendFileSync, realpathSync, writeFileSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const SHIPPED_PATHS = ['app', 'crates'];

const SEMVER_TAG_RE = /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const PR_MERGE_RE = /^Merge pull request #(\d+)\b/;
const BREAKING_HEADER_RE = /^[a-z][a-z0-9-]*(?:\([^)\r\n]+\))?!:\s+/i;
const BREAKING_BODY_RE = /^BREAKING[ -]CHANGE:\s*/m;
const MINOR_HEADER_RE = /^feat(?:\([^)\r\n]+\))?:\s+/i;
const SKIP_MARKER_RE = /\[skip release\]/i;
const VALID_BUMPS = new Set(['auto', 'patch', 'minor', 'major']);
const BUMP_RANK = { patch: 0, minor: 1, major: 2 };

export function parseSemver(tag) {
  const match = SEMVER_TAG_RE.exec(String(tag).trim());
  if (!match) return null;
  const major = Number(match[1]);
  const minor = Number(match[2]);
  const patch = Number(match[3]);
  return {
    tag: match[0],
    version: `${major}.${minor}.${patch}`,
    major,
    minor,
    patch,
  };
}

export function compareSemver(a, b) {
  return (a.major - b.major) || (a.minor - b.minor) || (a.patch - b.patch);
}

export function latestSemverTag(tags) {
  const parsed = tags.map(parseSemver).filter(Boolean);
  if (parsed.length === 0) return null;
  parsed.sort(compareSemver);
  return parsed[parsed.length - 1];
}

export function bumpVersion(previous, bump) {
  const base = previous ?? { major: 0, minor: 0, patch: 0 };
  if (bump === 'major') return `${base.major + 1}.0.0`;
  if (bump === 'minor') return `${base.major}.${base.minor + 1}.0`;
  if (bump === 'patch') return `${base.major}.${base.minor}.${base.patch + 1}`;
  throw new Error(`Unsupported bump type: ${bump}`);
}

export function extractCommitInfo(commit) {
  const subject = (commit.subject ?? '').trim();
  const rawBody = (commit.body ?? '').replace(/\r\n/g, '\n');
  const parents = Array.isArray(commit.parents)
    ? commit.parents.filter(Boolean)
    : String(commit.parents ?? '').trim().split(/\s+/).filter(Boolean);
  const prMatch = PR_MERGE_RE.exec(subject);

  if (prMatch) {
    const lines = rawBody.split('\n');
    const titleIdx = lines.findIndex((line) => line.trim().length > 0);
    const title = titleIdx >= 0 ? lines[titleIdx].trim() : subject;
    const body = titleIdx >= 0 ? lines.slice(titleIdx + 1).join('\n').trim() : '';
    return {
      hash: commit.hash ?? '',
      parents,
      title,
      body,
      prNumber: Number(prMatch[1]),
      isPrMerge: true,
      isLocalMerge: false,
    };
  }

  return {
    hash: commit.hash ?? '',
    parents,
    title: subject,
    body: rawBody.trim(),
    prNumber: null,
    isPrMerge: false,
    isLocalMerge: parents.length > 1,
  };
}

export function classifyCommit({ title, body }) {
  const cleanTitle = (title ?? '').trim();
  const cleanBody = (body ?? '').trim();
  const skip = SKIP_MARKER_RE.test(cleanTitle) || SKIP_MARKER_RE.test(cleanBody);

  let bump = 'patch';
  if (BREAKING_HEADER_RE.test(cleanTitle) || BREAKING_BODY_RE.test(cleanBody)) {
    bump = 'major';
  } else if (MINOR_HEADER_RE.test(cleanTitle)) {
    bump = 'minor';
  }
  return { bump, skip };
}

export function maxBump(bumps) {
  let result = 'patch';
  for (const bump of bumps) {
    if ((BUMP_RANK[bump] ?? -1) > BUMP_RANK[result]) {
      result = bump;
    }
  }
  return result;
}

export function formatReleaseBullet(info) {
  const title = (info.title ?? '')
    .replace(new RegExp(SKIP_MARKER_RE.source, 'gi'), '')
    .replace(/\s+/g, ' ')
    .trim();
  if (!title) return null;
  if (info.prNumber != null && !new RegExp(`\\(#${info.prNumber}\\)$`).test(title)) {
    return `${title} (#${info.prNumber})`;
  }
  return title;
}

export function buildUpdaterNotes(bullets) {
  const items = bullets.length > 0 ? bullets : ['Maintenance release'];
  return ['## What\'s new', ...items.map((item) => `- ${item}`)].join('\n');
}

export function buildReleaseNotes({
  version,
  previousTag = null,
  bullets = [],
  repository = '',
  notarized = true,
}) {
  const lines = [
    buildUpdaterNotes(bullets),
    '',
    `Download Voxi_${version}_universal.dmg and drag Voxi to Applications.`,
    'Existing users can use Settings → Check for updates.',
  ];
  if (!notarized) {
    lines.push(
      '',
      'This build is not notarized yet. The first time you open it, right-click Voxi in Applications and choose Open.',
    );
  }
  if (previousTag && repository) {
    lines.push(
      '',
      `Full changelog: https://github.com/${repository}/compare/${previousTag}...v${version}`,
    );
  }
  return `${lines.join('\n')}\n`;
}

function git(args, cwd) {
  return execFileSync('git', args, {
    cwd,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  });
}

function parseGitLog(raw) {
  const commits = [];
  for (const record of raw.split('\x1e')) {
    const trimmed = record.replace(/^\n+/, '');
    if (!trimmed) continue;
    const [hash = '', parents = '', subject = '', body = ''] = trimmed.split('\x1f');
    if (!hash.trim()) continue;
    commits.push({
      hash: hash.trim(),
      parents: parents.trim().split(/\s+/).filter(Boolean),
      subject: subject.trim(),
      body,
    });
  }
  return commits;
}

function readLog(cwd, args) {
  const raw = git(['log', ...args, '--format=%H%x1f%P%x1f%s%x1f%b%x1e'], cwd);
  return parseGitLog(raw);
}

function hasPathDiff(cwd, fromRef, toRef, paths) {
  try {
    execFileSync('git', ['diff', '--quiet', fromRef, toRef, '--', ...paths], {
      cwd,
      stdio: 'ignore',
    });
    return false;
  } catch (err) {
    if (err && err.status === 1) return true;
    throw err;
  }
}

function commitTouchesPaths(cwd, hash, firstParent, paths) {
  if (firstParent) {
    return hasPathDiff(cwd, firstParent, hash, paths);
  }
  const out = git(
    ['diff-tree', '--no-commit-id', '--name-only', '-r', '--root', hash, '--', ...paths],
    cwd,
  );
  return out.trim().length > 0;
}

function tagExists(cwd, tag) {
  try {
    execFileSync('git', ['rev-parse', '-q', '--verify', `refs/tags/${tag}`], {
      cwd,
      stdio: 'ignore',
    });
    return true;
  } catch {
    return false;
  }
}

export function computeReleasePlan({
  repoDir = process.cwd(),
  ref = 'HEAD',
  event = 'push',
  bump = 'auto',
  force = false,
  repository = '',
  notarized = true,
  paths = SHIPPED_PATHS,
} = {}) {
  const requestedBump = bump || 'auto';
  if (!VALID_BUMPS.has(requestedBump)) {
    throw new Error(`Invalid bump "${requestedBump}". Expected one of: auto, patch, minor, major.`);
  }

  const repoRoot = git(['rev-parse', '--show-toplevel'], repoDir).trim();
  const tags = git(['tag', '--list'], repoRoot)
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean);
  const latest = latestSemverTag(tags);
  const previousTag = latest ? latest.tag : null;
  const range = previousTag ? `${previousTag}..${ref}` : ref;
  const firstParentCommits = readLog(repoRoot, ['--first-parent', range]);

  const entries = firstParentCommits.map((commit) => {
    const info = extractCommitInfo(commit);
    let subEntries = [];
    if (info.isLocalMerge && info.parents.length >= 2) {
      subEntries = readLog(repoRoot, ['--no-merges', `${info.parents[0]}..${info.parents[1]}`])
        .map((sub) => {
          const subInfo = extractCommitInfo(sub);
          return { info: subInfo, classification: classifyCommit(subInfo) };
        });
    }
    const ownClassification = classifyCommit(info);
    const effectiveBump = maxBump([
      ownClassification.bump,
      ...subEntries.map((sub) => sub.classification.bump),
    ]);
    return {
      info,
      subEntries,
      classification: {
        bump: effectiveBump,
        skip: ownClassification.skip,
      },
    };
  });

  const forced = force || event === 'workflow_dispatch';
  const skipResult = (reason) => ({
    shouldRelease: false,
    reason,
    previousTag,
    version: null,
    tag: null,
    bump: null,
    bullets: [],
    updaterNotes: '',
    releaseNotes: '',
  });

  if (!forced) {
    if (previousTag && !hasPathDiff(repoRoot, previousTag, ref, paths)) {
      return skipResult(`No changes in ${paths.join('/ or ')}/ since ${previousTag}`);
    }
    if (entries.length > 0 && entries[0].classification.skip) {
      return skipResult('Skipped via [skip release] in HEAD');
    }
    if (previousTag && entries.length > 0) {
      const unskippedAppChange = entries.some(
        (entry) =>
          !entry.classification.skip &&
          commitTouchesPaths(repoRoot, entry.info.hash, entry.info.parents[0] ?? null, paths),
      );
      if (!unskippedAppChange) {
        return skipResult(
          `All ${paths.join('/ or ')}/ changes since ${previousTag} are marked [skip release]`,
        );
      }
    }
  }

  const chosenBump =
    requestedBump === 'auto'
      ? maxBump(entries.map((entry) => entry.classification.bump))
      : requestedBump;
  const version = bumpVersion(latest, chosenBump);
  const tag = `v${version}`;

  if (tagExists(repoRoot, tag)) {
    throw new Error(`Tag ${tag} already exists`);
  }

  const bullets = [];
  for (const entry of entries) {
    if (entry.info.isLocalMerge && entry.subEntries.length > 0) {
      for (const sub of entry.subEntries) {
        const bullet = formatReleaseBullet(sub.info);
        if (bullet) bullets.push(bullet);
      }
    } else {
      const bullet = formatReleaseBullet(entry.info);
      if (bullet) bullets.push(bullet);
    }
  }

  const updaterNotes = buildUpdaterNotes(bullets);
  const releaseNotes = buildReleaseNotes({
    version,
    previousTag,
    bullets,
    repository,
    notarized,
  });

  return {
    shouldRelease: true,
    reason: forced
      ? `Forced release (${event}, bump=${chosenBump})`
      : `Changes in ${paths.join('/ or ')}/ since ${previousTag ?? 'initial commit'} (bump=${chosenBump})`,
    previousTag,
    version,
    tag,
    bump: chosenBump,
    bullets,
    updaterNotes,
    releaseNotes,
  };
}

function parseBool(value, flag) {
  if (value === 'true' || value === '1') return true;
  if (value === 'false' || value === '0') return false;
  throw new Error(`Invalid boolean for ${flag}: ${value}`);
}

export function parseArgs(argv) {
  const options = {
    repoDir: process.cwd(),
    ref: 'HEAD',
    event: process.env.GITHUB_EVENT_NAME || 'push',
    bump: 'auto',
    force: false,
    json: false,
    requireRelease: false,
    githubOutput: null,
    writeNotes: null,
    writeUpdaterNotes: null,
    repository: process.env.GITHUB_REPOSITORY || '',
    notarized: true,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const next = () => {
      if (i + 1 >= argv.length) throw new Error(`Missing value for ${arg}`);
      i += 1;
      return argv[i];
    };
    switch (arg) {
      case '--repo':
        options.repoDir = next();
        break;
      case '--ref':
        options.ref = next();
        break;
      case '--event':
        options.event = next();
        break;
      case '--bump':
        options.bump = next() || 'auto';
        break;
      case '--force':
        options.force = true;
        break;
      case '--json':
        options.json = true;
        break;
      case '--require-release':
        options.requireRelease = true;
        break;
      case '--github-output':
        options.githubOutput = next();
        break;
      case '--write-notes':
        options.writeNotes = next();
        break;
      case '--write-updater-notes':
        options.writeUpdaterNotes = next();
        break;
      case '--repository':
        options.repository = next();
        break;
      case '--notarized':
        options.notarized = parseBool(next(), '--notarized');
        break;
      case '--help':
      case '-h':
        options.help = true;
        break;
      default:
        throw new Error(`Unknown argument: ${arg}`);
    }
  }
  return options;
}

function writeGithubOutput(filePath, plan) {
  const sanitize = (value) => String(value ?? '').replace(/[\r\n]+/g, ' ');
  const lines = [
    `should_release=${plan.shouldRelease}`,
    `version=${sanitize(plan.version)}`,
    `tag=${sanitize(plan.tag)}`,
    `previous_tag=${sanitize(plan.previousTag)}`,
    `bump=${sanitize(plan.bump)}`,
    `reason=${sanitize(plan.reason)}`,
  ];
  appendFileSync(filePath, `${lines.join('\n')}\n`, 'utf8');
}

export function main(argv = process.argv.slice(2)) {
  const options = parseArgs(argv);
  if (options.help) {
    console.log(
      [
        'Usage: next-version.mjs [options]',
        '',
        'Options:',
        '  --repo <dir>                 Git repository path (default: cwd)',
        '  --ref <ref>                  Target git ref (default: HEAD)',
        '  --event <name>               GitHub event name (default: GITHUB_EVENT_NAME or push)',
        '  --bump <auto|patch|minor|major>  Bump override (default: auto)',
        '  --force                      Force a release even when no shipped files changed',
        '  --require-release            Exit non-zero if should_release is false',
        '  --notarized <true|false>     Include or omit non-notarized Gatekeeper note (default: true)',
        '  --repository <owner/repo>    GitHub repo slug for changelog link',
        '  --github-output <path>       Append key=value outputs for GitHub Actions',
        '  --write-notes <path>         Write GitHub release notes Markdown to file',
        '  --write-updater-notes <path> Write updater latest.json notes Markdown to file',
        '  --json                       Print JSON plan to stdout',
      ].join('\n'),
    );
    return;
  }

  const plan = computeReleasePlan(options);

  if (options.githubOutput) {
    writeGithubOutput(options.githubOutput, plan);
  }
  if (options.requireRelease && !plan.shouldRelease) {
    throw new Error(`Release required, but skipped: ${plan.reason}`);
  }
  if (plan.shouldRelease) {
    if (options.writeNotes) {
      writeFileSync(options.writeNotes, plan.releaseNotes, 'utf8');
    }
    if (options.writeUpdaterNotes) {
      writeFileSync(options.writeUpdaterNotes, `${plan.updaterNotes}\n`, 'utf8');
    }
  }

  if (options.json) {
    console.log(JSON.stringify(plan, null, 2));
  } else if (plan.shouldRelease) {
    console.log(
      `Releasing ${plan.version} (previous: ${plan.previousTag ?? 'none'}, bump: ${plan.bump})`,
    );
    console.log(plan.updaterNotes);
  } else {
    console.log(`No release needed (previous: ${plan.previousTag ?? 'none'}): ${plan.reason}`);
  }
}

const isEntrypoint =
  Boolean(process.argv[1]) &&
  pathToFileURL(realpathSync(process.argv[1])).href ===
    pathToFileURL(realpathSync(fileURLToPath(import.meta.url))).href;

if (isEntrypoint) {
  try {
    main();
  } catch (err) {
    console.error(`next-version: ${err instanceof Error ? err.message : String(err)}`);
    process.exit(1);
  }
}
