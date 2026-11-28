import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { test } from "node:test";
import { run } from "./fold-command-requests.mjs";

const require = createRequire(import.meta.url);
const shared = require("../../website/api/_requests.js");
const { createHandler, ipKey } = require("../../website/api/command-request.js");
const { createHandler: createWrongResultHandler } = require("../../website/api/wrong-result.js");
const { wrongResults } = shared;

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
        const created = { ...data, number: Math.max(0, ...state.issues.map((item) => item.number)) + 1, state: "open", labels: data.labels.map((name) => ({ name })), locked: false };
        state.issues.push(created);
        return created;
      }
      const direction = url.searchParams.get("direction");
      const label = url.searchParams.get("labels");
      const open = state.issues.filter((item) => item.state === "open" &&
        (item.labels ?? [{ name: shared.LABEL }]).some(({ name }) => name === label)).sort((a, b) => (direction === "asc" ? a.number - b.number : b.number - a.number));
      return open;
    }
    const target = issue(Number(parts[1]));
    if (!target) return fail(404);
    if (parts[2] === "lock") {
      target.locked = true;
      return null;
    }
    if (parts[2] === "comments") {
      if (method === "POST") {
        const comment = { id: state.nextComment++, issue: target.number, body: data.body, user: { login: AUTHOR } };
        state.comments.push(comment);
        return comment;
      }
      const page = Number(url.searchParams.get("page") ?? 1);
      const size = Number(url.searchParams.get("per_page") ?? 30);
      return state.comments.filter((comment) => comment.issue === target.number).slice((page - 1) * size, page * size);
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
  assert.equal(ipKey("FE80::1%eth0"), "fe80:0:0:0::/64");
  assert.equal(ipKey("::FFFF:198.51.100.4"), "198.51.100.4");
  assert.equal(ipKey("2001:db8::192.0.2.1"), "2001:db8:0:0::/64");
  assert.equal(ipKey("2001:db8:1:2:3:4:5:6:7::8"), "2001:db8:1:2:3:4:5:6:7::8", "malformed input never throws");
  assert.equal(ipKey("1::2::3"), "1::2::3");

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

test("a rollover that fails midway never folds a request twice", async () => {
  const issues = tracked();
  issues[0].body = `${shared.HEADER}\n${"x".repeat(shared.BODY_LIMIT - 120)}`;
  const github = fakeGitHub({ issues });
  for (const command of ["fits in one", "needs the next issue"]) await call(endpoint(github), { command });
  const original = github.api;
  let failed = false;
  github.api = async (path, method, ...rest) => {
    // The new issue's first write fails once, after the old issue was saved.
    if (method === "PATCH" && path === "/issues/2" && !failed) {
      failed = true;
      throw new Error("GitHub returned 502");
    }
    return original(path, method, ...rest);
  };
  await assert.rejects(fold(github));
  github.api = original;
  await fold(github);
  const all = github.state.issues.map((issue) => issue.body).join("\n");
  assert.equal(all.match(/`fits in one`/g).length, 1);
  assert.equal(all.match(/`needs the next issue`/g).length, 1);
  assert.equal(github.state.comments.length, 0);
  assert.match(github.state.issues[0].body, /^Continued in \[#2\]/m);
});

test("issues the workflow opens are locked, and a missing rollover link is repaired", async () => {
  const github = fakeGitHub({ label: false });
  await fold(github);
  assert.equal(github.state.issues[0].locked, true);
  github.state.issues.push({ number: 2, title: "Command requests (2)", body: shared.HEADER, state: "open" });
  await fold(github);
  assert.match(github.state.issues[0].body, /^Continued in \[#2\]/m);
});

test("a large batch rolls over as often as it needs to", async () => {
  const github = fakeGitHub({ issues: tracked() });
  // Backticks need a longer fence, so these expand to about 1,400 characters each.
  const command = "`".repeat(290);
  for (let i = 0; i < 100; i++) {
    github.state.comments.push({ id: 5000 + i, issue: 1, user: { login: AUTHOR }, body: shared.encodeComment(`${command}${i}`, "n".repeat(500)) });
  }
  assert.deepEqual(await fold(github), { folded: 100, dropped: 0 });
  assert.ok(github.state.issues.length >= 3, `${github.state.issues.length} issues`);
  assert.equal(github.state.comments.length, 0);
  assert.ok(github.state.issues.every((issue) => issue.body.length <= shared.BODY_LIMIT));
});

test("the author match ignores case", async () => {
  const github = fakeGitHub({ issues: tracked() });
  await call(endpoint(github), { command: "tidy my desk" });
  await run({ github: github.api, author: "Ruck-Syu", repo: REPO });
  assert.match(github.state.issues[0].body, /`tidy my desk`/);
});

const report = {
  command: "Move the screenshots to Documents",
  outcome: "plan",
  actions: [{ kind: "move", count: 3 }, { kind: "create_folder", count: 1 }],
  note: "Keep them by month",
  appVersion: "1.2.3",
};
const wrongFold = (github) => run({ github: github.api, author: AUTHOR, repo: REPO, config: wrongResults });
const wrongEndpoint = (github, extra = {}) => createWrongResultHandler({
  env: { GITHUB_TOKEN_COMMAND_REQUESTS: "test-token", WRONG_RESULTS_ISSUE_NUMBER: "2" }, fetch: github.fetch, ...extra,
});

test("wrong results use a separate locked issue and fold by identical command and outcome", async () => {
  const github = fakeGitHub({ issues: tracked() });
  await wrongFold(github);
  const [commands, wrong] = github.state.issues;
  assert.equal(wrong.title, "Wrong results");
  assert.equal(wrong.locked, true);
  const handler = wrongEndpoint(github);
  const submissions = [
    report,
    { ...report, note: "Ignore last month", actions: [{ kind: "move", count: 2 }] },
    { ...report, outcome: "problem", note: "", actions: [] },
    { ...report, command: report.command.toLowerCase(), note: "" },
  ];
  const responses = await Promise.all(submissions.map((input) => call(handler, input)));
  assert.ok(responses.every(({ statusCode }) => statusCode === 200));
  assert.deepEqual(github.state.comments.map(({ issue }) => issue), [2, 2, 2, 2]);
  assert.ok(!github.state.calls.some((c) => c.startsWith("PATCH")));
  await fold(github);
  assert.equal(commands.body, shared.HEADER);
  assert.deepEqual(await wrongFold(github), { folded: 4, dropped: 0 });
  assert.match(wrong.body, /^- \[ \] `Move the screenshots to Documents` · `plan` \(\+2\)$/m);
  assert.match(wrong.body, /^- \[ \] `Move the screenshots to Documents` · `problem` \(\+1\)$/m);
  assert.match(wrong.body, /^- \[ \] `move the screenshots to documents` · `plan` \(\+1\)$/m);
  assert.match(wrong.body, /Planned actions: `create_folder ×1, move ×3` · Voxi `1.2.3`/);
  assert.match(wrong.body, /Planned actions: `move ×2`/);
  assert.match(wrong.body, /Note: `Keep them by month`/);
  assert.match(wrong.body, /Note: `Ignore last month`/);
  assert.equal(github.state.comments.length, 0);
  wrong.body = wrong.body.replace("- [ ]", "- [x]");
  await call(wrongEndpoint(github), report);
  assert.deepEqual(await wrongFold(github), { folded: 1, dropped: 0 });
  assert.match(wrong.body, /^- \[x\] `Move the screenshots to Documents` · `plan` \(\+2\)$/m);
  assert.match(wrong.body, /^- \[ \] `Move the screenshots to Documents` · `plan` \(\+1\)$/m);
});

test("wrong-result validation rejects private text, unknown fields and unbounded values", async () => {
  const github = fakeGitHub({ issues: tracked() });
  await wrongFold(github);
  const handler = wrongEndpoint(github);
  for (const invalid of [
    { ...report, command: "/Users/alex/Desktop" },
    { ...report, command: "C:\\Users\\alex" },
    { ...report, command: "move %252fUsers" },
    { ...report, note: "move ~/Downloads" },
    { ...report, note: "see folder／file" },
    { ...report, note: "private.pdf" },
    { ...report, note: "two\nlines" },
    { ...report, note: null },
    { ...report, searchStats: { path: "/Users/alex" } },
    { ...report, command: "x".repeat(301) },
    { ...report, note: "x".repeat(501) },
    { ...report, outcome: "private.pdf" },
    { ...report, outcome: "info" },
    { ...report, appVersion: "1.2.3\n" },
    { ...report, appVersion: "/Users/alex" },
    { ...report, actions: [{ kind: "move", count: 1, path: "/Users/alex" }] },
    { ...report, actions: [{ kind: "/Users/alex", count: 1 }] },
    { ...report, actions: [{ kind: "move", count: "1" }] },
    { ...report, actions: [{ kind: "move", count: Number.MAX_SAFE_INTEGER + 1 }] },
    { ...report, actions: [{ kind: "move", count: 1 }, { kind: "move", count: 2 }] },
  ]) {
    assert.equal((await call(handler, invalid)).statusCode, 400, JSON.stringify(invalid));
  }
  assert.equal(github.state.comments.length, 0);
  assert.equal((await call(handler, report)).statusCode, 200);
  const [comment] = github.state.comments;
  assert.deepEqual(wrongResults.decode(comment.body), { ...report, actions: [...report.actions].reverse() });

  github.state.comments.push({
    id: 3000, issue: 2, user: { login: AUTHOR }, body: wrongResults.encode({ ...report, note: "/Users/alex" }),
  });
  assert.deepEqual(await wrongFold(github), { folded: 1, dropped: 1 });
  assert.doesNotMatch(github.state.issues[1].body, /alex/);
});

test("wrong-result setup errors and failed GitHub writes use the feedback error contract", async () => {
  const github = fakeGitHub({ issues: tracked() });
  const missing = wrongEndpoint(github, { env: { GITHUB_TOKEN_COMMAND_REQUESTS: "test-token" } });
  assert.deepEqual((await call(missing, report)).body, { ok: false, error: "Wrong-result reports aren't set up yet." });
  assert.equal((await call(wrongEndpoint(github), report)).statusCode, 503);
  await wrongFold(github);
  assert.equal((await call(wrongEndpoint(github, {
    env: { GITHUB_TOKEN_COMMAND_REQUESTS: "test-token", WRONG_RESULTS_ISSUE_NUMBER: "1" },
  }), report)).statusCode, 503, "does not queue wrong results on the command-request issue");

  const refused = wrongEndpoint(github, {
    fetch: (href, options) => options.method === "POST" ?
      Promise.resolve(Response.json({}, { status: 403 })) : github.fetch(href, options),
  });
  const response = await call(refused, report);
  assert.equal(response.statusCode, 502);
  assert.deepEqual(response.body, { ok: false, error: "Couldn't send the report. Try again." });
});

test("wrong-result rollover keeps draining the configured issue and repairs a failed lock", async () => {
  const github = fakeGitHub({ issues: tracked() });
  await wrongFold(github);
  const base = github.state.issues[1];
  base.body += "\n" + "x".repeat(shared.BODY_LIMIT);
  const handler = wrongEndpoint(github);
  await call(handler, report);
  const request = github.api;
  github.api = async (path, method, ...args) => {
    if (path === "/issues/3/lock") throw new Error("lock interrupted");
    return request(path, method, ...args);
  };
  await assert.rejects(wrongFold(github), /lock interrupted/);
  github.api = request;
  await wrongFold(github);
  assert.equal(github.state.issues[2].locked, true);
  assert.equal(github.state.issues[2].title, "Wrong results (2)");
  assert.match(base.body, /^Continued in \[#3\]/);
  await call(handler, report);
  assert.equal(github.state.comments[0].issue, base.number);
  await wrongFold(github);
  assert.match(github.state.issues[2].body, /· `plan` \(\+2\)/);
});
