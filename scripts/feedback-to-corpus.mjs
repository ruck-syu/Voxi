import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, '..');

const HEADER =
  '# Inbox: reports to review. Replace each ? with the expected intent JSON or !, then move the line into eval/ or src/parse/corpus/.';

function expandHome(p) {
  if (p === '~') return os.homedir();
  if (p.startsWith('~/')) return path.join(os.homedir(), p.slice(2));
  return p;
}

function oneLine(value) {
  return String(value).replace(/\r\n|\r|\n/g, ' ').trim();
}

function localDate(d) {
  const pad = (n) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

// Returns the parsed report object, or null when the line is blank or invalid.
function parseReport(raw) {
  const line = raw.trim();
  if (!line) return null;
  let obj;
  try {
    obj = JSON.parse(line);
  } catch {
    return null;
  }
  if (!obj || typeof obj !== 'object' || Array.isArray(obj)) return null;
  if (typeof obj.command !== 'string' || !obj.command.trim()) return null;
  return obj;
}

const [inArg, outArg] = process.argv.slice(2);
if (!inArg) {
  console.error('usage: node scripts/feedback-to-corpus.mjs <reports.jsonl> [out.txt]');
  process.exit(2);
}

const inPath = path.resolve(expandHome(inArg));
const outPath = outArg
  ? path.resolve(expandHome(outArg))
  : path.join(repoRoot, 'app', 'src-tauri', 'eval', 'inbox', `reports-${localDate(new Date())}.txt`);

let text;
try {
  text = fs.readFileSync(inPath, 'utf8');
} catch (err) {
  console.error(`cannot read ${inPath}: ${err.message}`);
  process.exit(1);
}

const lines = text.split('\n');
if (lines.length && lines[lines.length - 1] === '') lines.pop();

let reportsRead = 0;
let linesSkipped = 0;
const groups = new Map();

for (const raw of lines) {
  const report = parseReport(raw);
  if (!report) {
    linesSkipped++;
    continue;
  }
  reportsRead++;

  const key = report.command.trim().toLowerCase();
  let group = groups.get(key);
  if (!group) {
    group = { latest: null, rank: -Infinity, notes: [] };
    groups.set(key, group);
  }

  // Keep the latest report by time; ties and unparseable times fall back to file order.
  const ms = typeof report.time === 'string' ? Date.parse(report.time) : NaN;
  const rank = Number.isNaN(ms) ? -Infinity : ms;
  if (rank >= group.rank) {
    group.latest = report;
    group.rank = rank;
  }

  if (typeof report.note === 'string') {
    const note = oneLine(report.note);
    if (note && !group.notes.includes(note)) group.notes.push(note);
  }
}

const entries = [];
for (const group of groups.values()) {
  const r = group.latest;
  const notes = [...group.notes];
  let command = oneLine(r.command);
  if (command.includes(' => ')) {
    command = command.split(' => ').join(' -> ');
    notes.push('command contained =>');
  }

  const status = r.outcome?.status ?? 'unknown';
  const parsed = r.intent == null ? 'refused' : oneLine(JSON.stringify(r.intent));

  const block = [
    `# report ${oneLine(r.time ?? 'unknown')} v${oneLine(r.version ?? 'unknown')} outcome=${oneLine(status)}`,
    ...notes.map((n) => `# note: ${n}`),
    `# parsed: ${parsed}`,
    `${command} => ?`,
  ];
  entries.push(block.join('\n'));
}

const body = `${HEADER}\n\n` + entries.map((e) => `${e}\n\n`).join('');

fs.mkdirSync(path.dirname(outPath), { recursive: true });
fs.writeFileSync(outPath, body, 'utf8');

console.log(`${reportsRead} reports read`);
console.log(`${groups.size} unique commands written`);
console.log(`${linesSkipped} lines skipped`);
console.log(`output: ${outPath}`);
