// Shared by the endpoint (which only queues requests as issue comments) and
// the GitHub Action that folds them into the checklist. Vercel does not turn
// files starting with "_" into functions.

const schema = require("./_wrong-result-schema.json");
const { URL_TEXT, wrongResultTextError } = require("./_wrong-result-text.mjs");

const LABEL = "command-requests";
const TITLE = "Command requests";
const HEADER = "Requests submitted from Voxi. Tick a command when Voxi supports it.";
const MARKER = "<!-- voxi-command-request:";
const BODY_LIMIT = 60_000;
const MAX_COMMAND = schema.maxCommand;
const MAX_NOTE = schema.maxNote;
const KEEP_RECEIPTS = 200;
const RECEIPT_LINE = /^<!-- voxi-folded: ([\d,]*) -->\r?$/;
// Cf covers bidi overrides and zero-width characters that could disguise text.
const CONTROL = /[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/u;
const CONTROL_GLOBAL = new RegExp(CONTROL.source, "gu");
const APP_VERSION = new RegExp(schema.versionPattern);
const TASK = /^([-*+] \[ \] )(.*?) \(\+([1-9]\d*)\)([ \t]*\r?)$/;

/** The trimmed command and note, or a user-facing error. */
function validate(rawCommand, rawNote = "") {
  if (typeof rawCommand !== "string" || typeof rawNote !== "string") {
    return { error: "Send the command and note as text." };
  }
  const command = rawCommand.trim();
  const note = rawNote.trim();
  if (!command || command.length > MAX_COMMAND || note.length > MAX_NOTE) {
    return { error: "Use 1–300 characters for the command and up to 500 for the note." };
  }
  if ([rawCommand, rawNote].some((text) => CONTROL.test(text) || URL_TEXT.test(text))) {
    return { error: "Remove links and line breaks from the command and note." };
  }
  return { command, note };
}

/** The report schema admits only user text, identifiers from fixed sets, and counts. */
function validateWrongResult(input) {
  if (!input || typeof input !== "object" || Object.keys(input).length !== 5 ||
      !["command", "outcome", "actions", "note", "appVersion"].every((key) => Object.hasOwn(input, key))) {
    return { error: "Send only the report fields shown in Voxi." };
  }
  const { command, outcome, actions, note, appVersion } = input;
  if (typeof command !== "string" || typeof note !== "string") {
    return { error: "Send the command and note as text." };
  }
  const error = wrongResultTextError(command, "command") ?? wrongResultTextError(note, "note");
  if (error) return { error };
  if (!schema.outcomes.includes(outcome) || typeof appVersion !== "string" || !APP_VERSION.test(appVersion) ||
      !Array.isArray(actions) || actions.length > schema.actionKinds.length ||
      ((outcome !== "plan" && outcome !== "done") && actions.length)) {
    return { error: "Send a valid outcome, action list and app version." };
  }
  const seen = new Set();
  const counts = [];
  for (const action of actions) {
    if (!action || typeof action !== "object" || Object.keys(action).length !== 2 ||
        !Object.hasOwn(action, "kind") || !Object.hasOwn(action, "count") ||
        !schema.actionKinds.includes(action.kind) || seen.has(action.kind) ||
        !Number.isSafeInteger(action.count) || action.count < 1) {
      return { error: "Send only known action kinds and positive counts." };
    }
    seen.add(action.kind);
    counts.push({ kind: action.kind, count: action.count });
  }
  counts.sort((a, b) => a.kind.localeCompare(b.kind));
  return { command, note, outcome, actions: counts, appVersion };
}

/** Base64 prevents user text from closing the HTML comment. */
function commentCodec(marker, validateInput) {
  return {
    encode(input) {
      const payload = Buffer.from(JSON.stringify(input), "utf8").toString("base64");
      return `${marker}${payload} -->\nQueued for the checklist.`;
    },
    decode(body) {
      if (typeof body !== "string" || !body.startsWith(marker)) return null;
      const end = body.indexOf(" -->", marker.length);
      if (end === -1) return null;
      try {
        const request = validateInput(JSON.parse(Buffer.from(body.slice(marker.length, end), "base64").toString("utf8")));
        return request.error ? null : request;
      } catch {
        return null;
      }
    },
  };
}

function validateCommandRequest(input) {
  return validate(input.command, input.note ?? "");
}

const commandCodec = commentCodec(MARKER, validateCommandRequest);
const wrongMarker = "<!-- voxi-wrong-result:";
const wrongCodec = commentCodec(wrongMarker, validateWrongResult);

function encodeComment(command, note) {
  return commandCodec.encode({ command, note });
}

function normalized(text) {
  return text.trim().replace(/\s+/gu, " ").toLowerCase();
}

function inlineCode(text) {
  text = text.replace(CONTROL_GLOBAL, "").replaceAll("@", "@​");
  const longestRun = Math.max(0, ...(text.match(/`+/g) ?? []).map((run) => run.length));
  const fence = "`".repeat(longestRun + 1);
  const content = text.startsWith("`") || text.endsWith("`") ? ` ${text} ` : text;
  return `${fence}${content}${fence}`;
}

/** Comment ids already folded into this body. */
function receipts(body) {
  return body.split("\n").flatMap((line) => line.match(RECEIPT_LINE)?.[1].split(",").filter(Boolean) ?? []);
}

function findOpenTask(lines, matches) {
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
    return task && matches(task[2]);
  });
}

/** Bump an open checkbox or append a fresh one, then record the comment id. */
function foldTask(body, title, matches, detail, id, { skipRepeatedDetail = false } = {}) {
  const lines = body.split("\n").filter((line) => !RECEIPT_LINE.test(line));
  const index = findOpenTask(lines, matches);
  if (index === -1) {
    lines.push("", `- [ ] ${title} (+1)`);
    if (detail) lines.push(detail);
  } else {
    const [, prefix, text, count, trailing] = lines[index].match(TASK);
    lines[index] = `${prefix}${text} (+${BigInt(count) + 1n})${trailing}`;
    if (detail && !(skipRepeatedDetail && lines[index + 1]?.replace(/\r$/, "") === detail)) {
      lines.splice(index + 1, 0, detail);
    }
  }
  const kept = [...receipts(body), id].slice(-KEEP_RECEIPTS);
  return `${lines.join("\n").replace(/\n+$/, "")}\n<!-- voxi-folded: ${kept.join(",")} -->`;
}

function fold(body, { command, note }, id) {
  const key = normalized(command);
  const matches = (title) => {
    const code = title.match(/^(`+)(.*?)\1$/);
    return code && normalized(code[2].replaceAll("@​", "@")) === key;
  };
  return foldTask(body, inlineCode(command), matches, note ? `  Note: ${inlineCode(note)}` : "", id);
}

function foldWrongResult(body, { command, outcome, actions, note, appVersion }, id) {
  const title = `${inlineCode(command)} · ${inlineCode(outcome)}`;
  const counts = actions.map(({ kind, count }) => `${kind} ×${count}`).join(", ") || "None";
  const noteText = note ? ` · Note: ${inlineCode(note)}` : "";
  const detail = `  - Planned actions: ${inlineCode(counts)} · Voxi ${inlineCode(appVersion)}${noteText}`;
  return foldTask(body, title, (text) => text === title, detail, id, { skipRepeatedDetail: true });
}

const commandRequests = {
  label: LABEL,
  title: TITLE,
  header: HEADER,
  marker: MARKER,
  ...commandCodec,
  fold,
  validateInput: validateCommandRequest,
  methodError: "Use POST to send a command request.",
  setupError: "Command requests aren't set up yet.",
  sendError: "Couldn't send the request. Try again.",
};

const wrongResults = {
  label: "wrong-results",
  title: "Wrong results",
  header: "Wrong results reported from Voxi. Tick a report when the result is fixed.",
  marker: wrongMarker,
  ...wrongCodec,
  fold: foldWrongResult,
  validateInput: validateWrongResult,
  issueEnv: "WRONG_RESULTS_ISSUE_NUMBER",
  methodError: "Use POST to send a wrong-result report.",
  setupError: "Wrong-result reports aren't set up yet.",
  sendError: "Couldn't send the report. Try again.",
};

const RATE_WINDOW = 10 * 60_000;
const PER_IP = 5;
// Each warm instance also caps all senders together. Cold starts reset both.
const PER_INSTANCE = 60;
const ISSUE_CACHE_MS = 5 * 60_000;

/** One key per IPv4 address, or per IPv6 /64 (one home network). */
function ipKey(raw) {
  const ip = raw.trim().replace(/%.*$/, "").toLowerCase(); // drop a zone id
  if (!ip.includes(":")) return ip;
  const mapped = ip.match(/^::ffff:(\d{1,3}(?:\.\d{1,3}){3})$/);
  if (mapped) return mapped[1];
  const halves = ip.split("::");
  const valid = /^[\da-f:.]+$/.test(ip) && halves.length <= 2;
  const left = halves[0] ? halves[0].split(":") : [];
  const right = halves.length === 2 && halves[1] ? halves[1].split(":") : [];
  const missing = 8 - left.length - right.length;
  if (!valid || (halves.length === 2 ? missing < 1 : missing !== 0)) return ip; // malformed: its own bucket
  const groups = [...left, ...Array(halves.length === 2 ? missing : 0).fill("0"), ...right];
  return `${groups.slice(0, 4).map((group) => group.replace(/^0+(?=.)/, "")).join(":")}::/64`;
}

function createHandler(config, { env = process.env, fetch = globalThis.fetch, now = Date.now } = {}) {
  const limits = new Map();
  let instance = { count: 0, until: 0 };
  let nextSweep = 0;
  let cachedIssue = null;

  return async function handler(req, res) {
    const fail = (status, error) => res.status(status).json({ ok: false, error });
    const success = () => res.status(200).json({ ok: true });
    if (req.method !== "POST") {
      res.setHeader("Allow", "POST");
      return fail(405, config.methodError);
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
    const repo = env.COMMAND_REQUESTS_REPO || "ruck-syu/Voxi";
    const configuredNumber = config.issueEnv && env[config.issueEnv];
    const number = Number(configuredNumber);
    if (!token || !/^[\w-]+\/[\w.-]+$/.test(repo) ||
        (config.issueEnv && (number < 1 || !Number.isSafeInteger(number) || String(number) !== configuredNumber))) {
      return fail(503, config.setupError);
    }

    const request = config.validateInput(input);
    if (request.error) return fail(400, request.error);

    const time = now();
    if (time >= nextSweep) {
      for (const [key, bucket] of limits) {
        if (time >= bucket.until) limits.delete(key);
      }
      nextSweep = time + RATE_WINDOW;
    }
    if (time >= instance.until) instance = { count: 0, until: time + RATE_WINDOW };
    // Vercel's edge sets this header; clients can't override it.
    const key = ipKey(req.headers["x-vercel-forwarded-for"]?.split(",")[0].trim() || req.socket?.remoteAddress || "unknown");
    let bucket = limits.get(key);
    if (!bucket || time >= bucket.until) {
      bucket = { count: 0, until: time + RATE_WINDOW };
      limits.set(key, bucket);
    }
    if (bucket.count >= PER_IP || instance.count >= PER_INSTANCE) {
      const until = bucket.count >= PER_IP ? bucket.until : instance.until;
      res.setHeader("Retry-After", String(Math.ceil((until - time) / 1000)));
      return fail(429, "Too many requests. Try again in a few minutes.");
    }
    bucket.count++;
    instance.count++;

    async function github(path, method = "GET", data) {
      const response = await fetch(`https://api.github.com/repos/${repo}${path}`, {
        method,
        headers: {
          Authorization: `Bearer ${token}`,
          Accept: "application/vnd.github+json",
          "Content-Type": "application/json",
          "User-Agent": "Voxi-command-requests",
        },
        body: data && JSON.stringify(data),
        redirect: "error",
        signal: AbortSignal.timeout(4000),
      });
      return response;
    }

    async function trackingIssue() {
      if (cachedIssue && time < cachedIssue.until) return cachedIssue.number;
      const response = await github(config.issueEnv
        ? `/issues/${number}`
        : `/issues?state=open&labels=${config.label}&sort=created&direction=desc&per_page=10`);
      if (config.issueEnv && [404, 410].includes(response.status)) return undefined;
      if (!response.ok) throw new Error(`GitHub returned ${response.status}`);
      const data = await response.json();
      const issue = config.issueEnv
        ? (data.number === number && !data.pull_request && data.state === "open" &&
           data.locked && data.labels?.some(({ name }) => name === config.label) ? data : null)
        : data.find((item) => !item.pull_request);
      cachedIssue = issue ? { number: issue.number, until: time + ISSUE_CACHE_MS } : null;
      return issue?.number;
    }

    try {
      for (let attempt = 0; attempt < (config.issueEnv ? 1 : 2); attempt++) {
        const number = await trackingIssue();
        if (!number) return fail(503, config.setupError);
        const response = await github(`/issues/${number}/comments`, "POST", { body: config.encode(request) });
        if (response.ok) return success();
        // The cached issue was closed, deleted or moved: look it up again once.
        if (![404, 410, 422].includes(response.status)) break;
        cachedIssue = null;
        if (config.issueEnv && response.status !== 422) return fail(503, config.setupError);
      }
    } catch {
      // Network errors and timeouts fall through to the generic message.
    }
    return fail(502, config.sendError);
  };
}

module.exports = {
  commandRequests,
  wrongResults,
  createHandler,
  ipKey,
  LABEL,
  TITLE,
  HEADER,
  MARKER,
  BODY_LIMIT,
  validate,
  encodeComment,
  decodeComment: commandCodec.decode,
  inlineCode,
  receipts,
  fold,
};
