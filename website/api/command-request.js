const { randomUUID } = require("node:crypto");
const { setTimeout: delay } = require("node:timers/promises");

const LABEL = "command-requests";
const TITLE = "Command requests";
const HEADER = "Requests submitted from Voxi. Tick a command when Voxi supports it.";
const BODY_LIMIT = 60_000;
const RATE_WINDOW = 10 * 60_000;
const RETRIES = 3;
const RECEIPT_LINE = /^<!-- voxi-request-receipts: ([a-f0-9,-]+) -->\r?$/;
const CONTROL = /[\p{Cc}\p{Zl}\p{Zp}]/u;
const CONTROL_GLOBAL = new RegExp(CONTROL.source, "gu");
// Include bare hosts that look like links without treating filenames like report.pdf as links.
const URL = /[a-z][a-z\d+.-]*:\/\/|\b(?:https?|ftp|file|mailto|tel|data|javascript):|\bwww\.|\b[\w-]+\.(?:com|org|net|edu|gov|io|dev|app|info|biz|co|uk|me|ai|xyz|test|example|local)\b|(?:^|[\s(<])\/\/[\w\d-]+|\b(?:\d{1,3}\.){3}\d{1,3}(?=[:/])/i;
const TASK = /^([-*+] \[ \] )(`+)(.*?)\2 \(\+([1-9]\d*)\)([ \t]*\r?)$/;

function normalized(text) {
  return text.trim().replace(/\s+/gu, " ").toLowerCase();
}

function inlineCode(text) {
  text = text.replace(CONTROL_GLOBAL, "").replaceAll("@", "@\u200b");
  const longestRun = Math.max(0, ...(text.match(/`+/g) ?? []).map((run) => run.length));
  const fence = "`".repeat(longestRun + 1);
  const content = text.startsWith("`") || text.endsWith("`") ? ` ${text} ` : text;
  return `${fence}${content}${fence}`;
}

function receipts(body) {
  return body.split("\n").flatMap((line) => line.match(RECEIPT_LINE)?.[1].split(",") ?? []);
}

function findOpenTask(lines, key) {
  let codeFence = "";
  return lines.findIndex((line) => {
    const fence = line.match(/^ {0,3}(`{3,}|~{3,})(.*)$/);
    if (codeFence) {
      if (fence && fence[1].startsWith(codeFence) && !fence[2].trim()) codeFence = "";
      return false;
    }
    if (fence && (fence[1][0] === "~" || !fence[2].includes("`"))) {
      codeFence = fence[1];
      return false;
    }
    const task = line.match(TASK);
    return task && normalized(task[3].replaceAll("@\u200b", "@")) === key;
  });
}

function withRequest(body, command, note, receipt) {
  const lines = body.split("\n").filter((line) => !RECEIPT_LINE.test(line));
  const index = findOpenTask(lines, normalized(command));
  const noteLine = note ? [`  Note: ${inlineCode(note)}`] : [];
  if (index === -1) {
    lines.push("", `- [ ] ${inlineCode(command)} (+1)`, ...noteLine);
  } else {
    const [, prefix, fence, text, count, trailing] = lines[index].match(TASK);
    lines.splice(index, 1, `${prefix}${fence}${text}${fence} (+${BigInt(count) + 1n})${trailing}`, ...noteLine);
  }
  // Receipts distinguish concurrent writes that calculate the same counter.
  const recent = [...receipts(body).slice(-31), receipt];
  return `${lines.join("\n")}\n<!-- voxi-request-receipts: ${recent.join(",")} -->`;
}

function retryDelay() {
  return delay(25 + Math.random() * 100);
}

async function saveRequest(github, repo, command, note) {
  const receipt = randomUUID();
  const rollovers = new Map();
  for (let attempt = 0; attempt <= RETRIES; attempt++) {
    if (attempt) await retryDelay();
    // The repository issue list can also contain pull requests.
    let latest;
    for (let page = 1; !latest; page++) {
      const issues = await github(`/issues?state=open&labels=${LABEL}&sort=created&direction=desc&per_page=100&page=${page}`);
      latest = issues.find((issue) => !issue.pull_request);
      if (issues.length < 100) break;
    }
    let issue = latest && await github(`/issues/${latest.number}`);
    if (issue && issue.state !== "open") continue;

    // Repair a rollover link before accepting the request, even on a retry.
    if (issue && rollovers.has(issue.number)) {
      await linkRollover(github, repo, rollovers.get(issue.number), issue.number);
    }
    const body = issue?.body ?? "";
    if (receipts(body).includes(receipt)) return;
    const updated = withRequest(body, command, note, receipt);
    if (!issue || updated.length > BODY_LIMIT) {
      if (!issue) await ensureLabel(github);
      const previous = issue?.number;
      const part = issue?.title.match(/^Command requests \((\d+)\)$/)?.[1] ?? 1;
      issue = await github("/issues", "POST", {
        title: previous ? `${TITLE} (${Number(part) + 1})` : TITLE,
        labels: [LABEL],
        body: withRequest(HEADER, command, note, receipt),
      });
      if (previous) rollovers.set(issue.number, previous);
    } else {
      await github(`/issues/${issue.number}`, "PATCH", { body: updated });
    }

    const verified = await github(`/issues/${issue.number}`);
    if (rollovers.has(issue.number)) {
      await linkRollover(github, repo, rollovers.get(issue.number), issue.number);
    }
    if (receipts(verified.body ?? "").includes(receipt)) return;
  }
  throw new Error("Could not verify the issue update");
}

async function linkRollover(github, repo, from, to) {
  const path = `/issues/${from}`;
  const link = `Continued in [#${to}](https://github.com/${repo}/issues/${to}).`;
  for (let attempt = 0; attempt <= RETRIES; attempt++) {
    if (attempt) await retryDelay();
    const issue = await github(path);
    const body = issue.body ?? "";
    if (body.split("\n").includes(link)) return;
    await github(path, "PATCH", { body: `${link}\n\n${body}` });
    const verified = await github(path);
    if ((verified.body ?? "").split("\n").includes(link)) return;
  }
  throw new Error("Could not verify the rollover link");
}

async function ensureLabel(github) {
  if (await github(`/labels/${LABEL}`, "GET", undefined, 404)) return;
  const label = await github("/labels", "POST", {
    name: LABEL,
    color: "1d76db",
    description: "Commands requested from Voxi",
  }, 422);
  if (!label) await github(`/labels/${LABEL}`);
}

function createHandler({ env = process.env, fetch = globalThis.fetch, now = Date.now } = {}) {
  const limits = new Map();
  let nextSweep = 0;
  let pendingWrite = Promise.resolve();

  return async function handler(req, res) {
    const fail = (status, error) => res.status(status).json({ ok: false, error });
    const success = () => res.status(200).json({ ok: true });
    if (req.method !== "POST") {
      res.setHeader("Allow", "POST");
      return fail(405, "Use POST to send a command request.");
    }
    if (!/^application\/json(?:\s*;|$)/i.test(req.headers["content-type"] ?? "")) {
      return fail(415, "Send the request as JSON.");
    }
    let input;
    try {
      input = req.body;
    } catch {
      return fail(400, "Send a valid JSON object.");
    }
    if (!input || typeof input !== "object" || Array.isArray(input)) {
      return fail(400, "Send a valid JSON object.");
    }
    if (Object.hasOwn(input, "website")) return success();

    const token = env.GITHUB_TOKEN_COMMAND_REQUESTS;
    if (!token) return fail(503, "Command requests aren't set up yet.");
    const repo = env.COMMAND_REQUESTS_REPO || "ruck-syu/Voxi";
    if (!/^[\w-]+\/[\w.-]+$/.test(repo)) return fail(503, "Command requests aren't set up yet.");

    const { command: rawCommand, note: rawNote = "" } = input;
    if (typeof rawCommand !== "string" || typeof rawNote !== "string") {
      return fail(400, "Send the command and note as text.");
    }
    const command = rawCommand.trim();
    const note = rawNote.trim();
    if (!command || command.length > 300 || note.length > 500) {
      return fail(400, "Use 1–300 characters for the command and up to 500 for the note.");
    }
    if ([rawCommand, rawNote].some((text) => CONTROL.test(text) || URL.test(text))) {
      return fail(400, "Remove links and line breaks from the command and note.");
    }

    const time = now();
    if (time >= nextSweep) {
      for (const [ip, bucket] of limits) {
        if (time >= bucket.until) limits.delete(ip);
      }
      nextSweep = time + RATE_WINDOW;
    }
    const ip = req.headers["x-vercel-forwarded-for"]?.split(",")[0].trim()
      || req.socket?.remoteAddress || "unknown";
    let bucket = limits.get(ip);
    if (!bucket || time >= bucket.until) {
      bucket = { count: 0, until: time + RATE_WINDOW };
      limits.set(ip, bucket);
    }
    if (bucket.count >= 5) {
      res.setHeader("Retry-After", String(Math.ceil((bucket.until - time) / 1000)));
      return fail(429, "Too many requests. Try again in a few minutes.");
    }
    bucket.count++;

    async function github(path, method = "GET", data, allowedStatus) {
      const response = await fetch(`https://api.github.com/repos/${repo}${path}`, {
        method,
        headers: {
          Authorization: `Bearer ${token}`,
          Accept: "application/vnd.github+json",
          "Content-Type": "application/json",
          "Cache-Control": "no-cache",
          "User-Agent": "Voxi-command-requests",
        },
        body: data && JSON.stringify(data),
        redirect: "error",
        signal: AbortSignal.timeout(4000),
      });
      if (response.status === allowedStatus) return null;
      if (!response.ok) throw new Error(`GitHub returned ${response.status}`);
      return response.json();
    }

    // Serialize this instance's writes. Verification and retry cover races across instances.
    const write = pendingWrite.then(() => saveRequest(github, repo, command, note));
    pendingWrite = write.catch(() => {});
    try {
      await write;
      return success();
    } catch {
      return fail(502, "Couldn't send the request. Try again.");
    }
  };
}

module.exports = createHandler();
module.exports.createHandler = createHandler;
