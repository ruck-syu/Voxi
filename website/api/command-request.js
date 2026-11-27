// Queues a command request as a comment on the tracking issue. Comments are
// append-only, so concurrent requests can't overwrite each other or the
// owner's ticks. The "Command requests" GitHub Action folds them into the
// checklist one run at a time (.github/scripts/fold-command-requests.mjs).
const { LABEL, validate, encodeComment } = require("./_requests.js");

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

function createHandler({ env = process.env, fetch = globalThis.fetch, now = Date.now } = {}) {
  const limits = new Map();
  let instance = { count: 0, until: 0 };
  let nextSweep = 0;
  let cachedIssue = null;

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
    const repo = env.COMMAND_REQUESTS_REPO || "ruck-syu/Voxi";
    if (!token || !/^[\w-]+\/[\w.-]+$/.test(repo)) return fail(503, "Command requests aren't set up yet.");

    const request = validate(input.command, input.note ?? "");
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
      const response = await github(`/issues?state=open&labels=${LABEL}&sort=created&direction=desc&per_page=10`);
      if (!response.ok) throw new Error(`GitHub returned ${response.status}`);
      const issue = (await response.json()).find((item) => !item.pull_request);
      cachedIssue = issue ? { number: issue.number, until: time + ISSUE_CACHE_MS } : null;
      return issue?.number;
    }

    try {
      for (let attempt = 0; attempt < 2; attempt++) {
        const number = await trackingIssue();
        if (!number) return fail(503, "Command requests aren't set up yet.");
        const response = await github(`/issues/${number}/comments`, "POST", { body: encodeComment(request.command, request.note) });
        if (response.ok) return success();
        // The cached issue was closed, deleted or moved: look it up again once.
        if (![404, 410, 422].includes(response.status)) break;
        cachedIssue = null;
      }
    } catch {
      // Network errors and timeouts fall through to the generic message.
    }
    return fail(502, "Couldn't send the request. Try again.");
  };
}

module.exports = createHandler();
module.exports.createHandler = createHandler;
module.exports.ipKey = ipKey;
