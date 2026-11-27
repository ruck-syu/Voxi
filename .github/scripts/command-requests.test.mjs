import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { test } from "node:test";
import { run } from "./fold-command-requests.mjs";

const require = createRequire(import.meta.url);
const shared = require("../../website/api/_requests.js");
const { createHandler, ipKey } = require("../../website/api/command-request.js");

const REPO = "ruck-syu/Voxi";
const AUTHOR = "ruck-syu";

/** An in-memory GitHub with issues, comments and labels. */
function fakeGitHub({ issues = [], label = true } = {}) {
  const state = { issues: structuredClone(issues), comments: [], label, nextComment: 1000, calls: [] };
  const issue = (number) => state.issues.find((item) => item.number === number);
  const api = async (path, method = "GET", data, allowedStatus) => {
    state.calls.push(`${method} ${path}`);
    const url = new URL(`https://x${path}`);
    const parts = url.pathname.split("/").filter(Boolean);
    if (parts[0] === "labels") {
      if (method === "GET") return state.label ? { name: shared.LABEL } : allowedStatus === 404 ? null : fail(404);
      state.label = true;
      return data;
    }
    if (parts[0] === "issues" && parts[1] === "comments") {
      const index = state.comments.findIndex((comment) => String(comment.id) === parts[2]);
      if (index === -1) return allowedStatus === 404 ? null : fail(404);
      state.comments.splice(index, 1);
      return null;
    }
    if (parts[0] === "issues" && parts.length === 1) {
      if (method === "POST") {
        const created = { ...data, number: Math.max(0, ...state.issues.map((item) => item.number)) + 1, state: "open", labels: [{ name: shared.LABEL }] };
        state.issues.push(created);
        return created;
      }
      const direction = url.searchParams.get("direction");
      const open = state.issues.filter((item) => item.state === "open").sort((a, b) => (direction === "asc" ? a.number - b.number : b.number - a.number));
      return open;
    }
    const target = issue(Number(parts[1]));
    if (!target) return fail(404);
    if (parts[2] === "comments") {
      if (method === "POST") {
        const comment = { id: state.nextComment++, issue: target.number, body: data.body, user: { login: AUTHOR } };
        state.comments.push(comment);
        return comment;
      }
      return state.comments.filter((comment) => comment.issue === target.number);
    }
    if (method === "PATCH") {
      state.beforePatch?.(target);
      Object.assign(target, data);
      return target;
    }
    return structuredClone(target);
  };
  const fail = (status) => {
    throw Object.assign(new Error(`GitHub returned ${status}`), { status });
  };
  // The endpoint calls fetch, not the client helper.
  const fetch = async (href, options) => {
    const path = new URL(href).pathname.slice(`/repos/${REPO}`.length) + new URL(href).search;
    try {
      const body = await api(path, options.method, options.body && JSON.parse(options.body));
      return Response.json(body ?? {}, { status: options.method === "POST" ? 201 : 200 });
    } catch (error) {
      return Response.json({}, { status: error.status ?? 500 });
    }
  };
  return { state, api, fetch };
}

function call(handler, body, headers = {}) {
  const res = {
    statusCode: 0,
    headers: {},
    status(code) { this.statusCode = code; return this; },
    json(data) { this.body = data; return this; },
    setHeader(name, value) { this.headers[name] = value; },
  };
  return handler({ method: "POST", headers: { "content-type": "application/json", "x-vercel-forwarded-for": "203.0.113.7", ...headers }, body }, res).then(() => res);
}

const tracked = () => [{ number: 1, title: "Command requests", body: shared.HEADER, state: "open" }];
const endpoint = (github, extra = {}) => createHandler({ env: { GITHUB_TOKEN_COMMAND_REQUESTS: "test-token" }, fetch: github.fetch, ...extra });
const fold = (github) => run({ github: github.api, author: AUTHOR, repo: REPO });

test("requests are queued as comments, then folded with counts and notes", async () => {
  const github = fakeGitHub({ issues: tracked() });
  const handler = endpoint(github);
  assert.equal((await call(handler, { command: "make one PDF from scans", note: "weekly" })).statusCode, 200);
  assert.equal((await call(handler, { command: "Make  one pdf FROM scans" })).statusCode, 200);
  assert.equal((await call(handler, { command: "tidy my desk" })).statusCode, 200);
  assert.equal(github.state.comments.length, 3);
  assert.ok(!github.state.calls.some((c) => c.startsWith("PATCH")), "the endpoint never edits the issue");

  assert.deepEqual(await fold(github), { folded: 3, dropped: 0 });
  const body = github.state.issues[0].body;
  assert.match(body, /^- \[ \] `make one PDF from scans` \(\+2\)$/m);
  assert.match(body, /^ {2}Note: `weekly`$/m);
  assert.match(body, /^- \[ \] `tidy my desk` \(\+1\)$/m);
  assert.equal(github.state.comments.length, 0, "folded comments are deleted");
  assert.deepEqual(await fold(github), { folded: 0, dropped: 0 }, "a second run is a no-op");
  assert.equal(github.state.issues[0].body, body);
});

test("a repeat of a ticked command adds a new line and owner edits are kept", async () => {
  const issues = tracked();
  issues[0].body = `${shared.HEADER}\n\nOwner's own text.\n\n- [x] \`tidy my desk\` (+4)`;
  const github = fakeGitHub({ issues });
  await call(endpoint(github), { command: "tidy my desk" });
  await fold(github);
  const body = github.state.issues[0].body;
  assert.match(body, /^Owner's own text\.$/m);
  assert.match(body, /^- \[x\] `tidy my desk` \(\+4\)$/m);
  assert.match(body, /^- \[ \] `tidy my desk` \(\+1\)$/m);
});

test("a run that fails before deleting comments never double-counts on retry", async () => {
  const github = fakeGitHub({ issues: tracked() });
  await call(endpoint(github), { command: "tidy my desk" });
  const remove = github.api;
  github.api = async (path, method, ...rest) => {
    if (method === "DELETE") throw new Error("network down");
    return remove(path, method, ...rest);
  };
  await assert.rejects(fold(github));
  github.api = remove;
  await fold(github);
  assert.match(github.state.issues[0].body, /`tidy my desk` \(\+1\)/);
  assert.equal(github.state.comments.length, 0);
});

test("an overwritten update is detected and retried by the next run", async () => {
  const github = fakeGitHub({ issues: tracked() });
  await call(endpoint(github), { command: "tidy my desk" });
  // The owner saves an older copy of the body right after the fold writes.
  github.state.beforePatch = (issue) => {
    github.state.beforePatch = null;
    queueMicrotask(() => { issue.body = shared.HEADER; });
  };
  const original = github.api;
  github.api = async (path, method, ...rest) => {
    const result = await original(path, method, ...rest);
    if (method === "PATCH") github.state.issues[0].body = shared.HEADER;
    return result;
  };
  await assert.rejects(fold(github), /verify/);
  assert.equal(github.state.comments.length, 1, "the request stays queued");
  github.api = original;
  await fold(github);
  assert.match(github.state.issues[0].body, /`tidy my desk` \(\+1\)/);
});

test("a full issue rolls over once, links to the next one and keeps going", async () => {
  const issues = tracked();
  issues[0].body = `${shared.HEADER}\n${"x".repeat(shared.BODY_LIMIT - 60)}`;
  const github = fakeGitHub({ issues });
  for (const command of ["first request", "second request"]) await call(endpoint(github), { command });
  await fold(github);
  const [old, next] = github.state.issues;
  assert.equal(next.title, "Command requests (2)");
  assert.match(old.body, /^Continued in \[#2\]\(https:\/\/github.com\/ruck-syu\/Voxi\/issues\/2\)\.$/m);
  assert.match(next.body, /`first request` \(\+1\)/);
  assert.match(next.body, /`second request` \(\+1\)/);
  assert.equal(github.state.issues.length, 2);
});

test("the first run creates the label and the tracking issue", async () => {
  const github = fakeGitHub({ label: false });
  await fold(github);
  assert.equal(github.state.label, true);
  assert.equal(github.state.issues[0].title, "Command requests");
  const res = await call(endpoint(github), { command: "tidy my desk" });
  assert.equal(res.statusCode, 200);
});

test("the endpoint reports setup problems instead of creating issues", async () => {
  const github = fakeGitHub();
  assert.equal((await call(endpoint(github), { command: "tidy my desk" })).statusCode, 503);
  assert.equal(github.state.issues.length, 0);
  const unset = createHandler({ env: {}, fetch: github.fetch });
  assert.deepEqual((await call(unset, { command: "x" })).body, { ok: false, error: "Command requests aren't set up yet." });
});

test("only the owner's marker comments are folded, and forged payloads are re-validated", async () => {
  const github = fakeGitHub({ issues: tracked() });
  github.state.comments.push(
    { id: 1, issue: 1, user: { login: "someone-else" }, body: shared.encodeComment("spam", "") },
    { id: 2, issue: 1, user: { login: AUTHOR }, body: "A normal owner comment." },
    { id: 3, issue: 1, user: { login: AUTHOR }, body: `${shared.MARKER}${Buffer.from(JSON.stringify({ command: "see https://evil.example" })).toString("base64")} -->` },
  );
  assert.deepEqual(await fold(github), { folded: 0, dropped: 1 });
  assert.doesNotMatch(github.state.issues[0].body, /spam|evil/);
  assert.deepEqual(github.state.comments.map((comment) => comment.id), [1, 2]);
});

test("text cannot inject Markdown, mentions, links or invisible characters", () => {
  const ok = (command) => assert.ok(!shared.validate(command).error, command);
  const bad = (command) => assert.ok(shared.validate(command).error, JSON.stringify(command));
  for (const command of ["move Safari.app to Applications", "group logo.ai files", "split report.pdf"]) ok(command);
  for (const command of ["see https://x.com", "www.example", "evil.com", "a\nb", "move ‮selif", "​", "sort​ files", "h​ttps://evil.com"]) bad(command);
  assert.equal(shared.inlineCode("hi @team"), "`hi @​team`");
  assert.equal(shared.inlineCode("a `b` c"), "``a `b` c``");
  assert.equal(shared.inlineCode("`edge`"), "`` `edge` ``");
  const body = shared.fold(shared.HEADER, { command: "- [x] `done` (+1)", note: "" }, "1");
  assert.match(body, /^- \[ \] ``- \[x\] `done` \(\+1\)`` \(\+1\)$/m, "a forged task stays inside code");
});

test("honeypot, method, content type and validation", async () => {
  const github = fakeGitHub({ issues: tracked() });
  const handler = endpoint(github);
  assert.equal((await call(handler, { command: "x", website: "bot" })).statusCode, 200);
  assert.equal(github.state.comments.length, 0);
  assert.equal((await call(handler, { command: "" })).statusCode, 400);
  assert.equal((await call(handler, { command: "x".repeat(301) })).statusCode, 400);
  assert.equal((await call(handler, { command: 5 })).statusCode, 400);
  assert.equal((await call(handler, "x", { "content-type": "text/plain" })).statusCode, 415);
});

test("rate limits group an IPv6 /64 and cap each instance", async () => {
  assert.equal(ipKey("2001:db8:1:2:aaaa::1"), "2001:db8:1:2::/64");
  assert.equal(ipKey("2001:0db8:0001:0002:ffff:ffff:ffff:ffff"), "2001:db8:1:2::/64");
  assert.equal(ipKey("::ffff:198.51.100.4"), "198.51.100.4");
  assert.equal(ipKey("198.51.100.4"), "198.51.100.4");

  const github = fakeGitHub({ issues: tracked() });
  const handler = endpoint(github, { now: () => 1_000_000 });
  for (let i = 0; i < 5; i++) {
    assert.equal((await call(handler, { command: `c${i}` }, { "x-vercel-forwarded-for": `2001:db8:1:2::${i + 1}` })).statusCode, 200);
  }
  const limited = await call(handler, { command: "c5" }, { "x-vercel-forwarded-for": "2001:db8:1:2:ffff::9" });
  assert.equal(limited.statusCode, 429);
  assert.ok(Number(limited.headers["Retry-After"]) > 0);

  const busy = endpoint(fakeGitHub({ issues: tracked() }), { now: () => 1_000_000 });
  let accepted = 0;
  for (let i = 0; i < 80; i++) {
    if ((await call(busy, { command: `c${i}` }, { "x-vercel-forwarded-for": `198.51.100.${i}` })).statusCode === 200) accepted++;
  }
  assert.equal(accepted, 60);
});

test("the endpoint never leaks the token or the request in errors", async () => {
  const handler = createHandler({ env: { GITHUB_TOKEN_COMMAND_REQUESTS: "secret-token" }, fetch: async () => { throw new Error("secret-token tidy"); } });
  const res = await call(handler, { command: "tidy my desk" });
  assert.equal(res.statusCode, 502);
  assert.doesNotMatch(JSON.stringify(res.body), /secret|tidy/);
});
