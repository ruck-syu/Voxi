import assert from "node:assert/strict";
import { test } from "node:test";
import endpoint from "../../website/api/command-request.js";

function mockGitHub({ issues = [], label = true, afterPatch } = {}) {
  issues = structuredClone(issues);
  const calls = [];
  const fetch = async (url, options) => {
    const { origin, pathname, searchParams } = new URL(url);
    const body = options.body && JSON.parse(options.body);
    assert.equal(origin, "https://api.github.com");
    assert.equal(options.headers.Authorization, "Bearer test-token");
    assert.ok(pathname.startsWith("/repos/ruck-syu/Voxi"));
    const path = pathname.slice("/repos/ruck-syu/Voxi".length);
    calls.push({ path, method: options.method });
    if (path === "/labels/command-requests") {
      return Response.json({ name: "command-requests" }, { status: label ? 200 : 404 });
    }
    if (path === "/labels" && options.method === "POST") {
      assert.equal(body.name, "command-requests");
      label = true;
      return Response.json(body, { status: 201 });
    }
    if (path === "/issues" && options.method === "GET") {
      assert.equal(searchParams.get("state"), "open");
      assert.equal(searchParams.get("labels"), "command-requests");
      assert.equal(searchParams.get("sort"), "created");
      assert.equal(searchParams.get("direction"), "desc");
      return Response.json(issues.filter((issue) => issue.state === "open").sort((a, b) => b.number - a.number));
    }
    if (path === "/issues" && options.method === "POST") {
      assert.ok(label);
      assert.deepEqual(body.labels, ["command-requests"]);
      const issue = { ...body, number: Math.max(0, ...issues.map(({ number }) => number)) + 1, state: "open" };
      issues.push(issue);
      return Response.json(issue, { status: 201 });
    }
    const issue = issues.find(({ number }) => path === `/issues/${number}`);
    assert.ok(issue, `Unexpected GitHub call: ${path}`);
    if (options.method === "GET") return Response.json(issue);
    assert.equal(options.method, "PATCH");
    assert.deepEqual(Object.keys(body), ["body"]);
    const before = issue.body;
    issue.body = body.body;
    const response = Response.json(issue);
    afterPatch?.(issue, before);
    return response;
  };
  return { issues, calls, fetch };
}

function harness({ now, env = { GITHUB_TOKEN_COMMAND_REQUESTS: "test-token" }, ...githubOptions } = {}) {
  const github = mockGitHub(githubOptions);
  const handler = endpoint.createHandler({ now, env, fetch: github.fetch });
  async function submit(body, { ip = "192.0.2.1", method = "POST", contentType = "application/json" } = {}) {
    const response = {
      headers: {},
      setHeader(name, value) { this.headers[name] = value; },
      status(code) { this.code = code; return this; },
      json(data) { this.data = data; },
    };
    await handler({ method, headers: { "content-type": contentType, "x-vercel-forwarded-for": ip }, body }, response);
    return response;
  }
  return { ...github, submit };
}

function openIssue(body, number = 1, title = "Command requests") {
  return { body, number, title, state: "open" };
}

test("creates a labelled tracker and escapes both command and note as inline code", async () => {
  const { submit, issues } = harness({ label: false });
  const response = await submit({
    command: "  ` + [x] # <b> @owner ``  ",
    note: "  [new task] <script> @someone `here`  ",
  });
  assert.equal(response.code, 200);
  assert.deepEqual(response.data, { ok: true });
  assert.equal(issues.length, 1);
  assert.equal(issues[0].title, "Command requests");
  assert.match(issues[0].body, /Tick a command/);
  assert.ok(issues[0].body.includes("- [ ] ``` ` + [x] # <b> @\u200bowner `` ``` (+1)"));
  assert.ok(issues[0].body.includes("  Note: `` [new task] <script> @\u200bsomeone `here` ``"));
});

test("increments normalized duplicates and keeps the notes and owner text", async () => {
  const owner = "# Owner's priorities\n\n- [x] `Copy photos` (+4)\n\nKeep this paragraph.";
  const { submit, issues } = harness({ issues: [openIssue(owner)] });
  await submit({ command: "  Sort   @work by date ", note: "Oldest first" });
  const response = await submit({ command: "SORT @WORK by date", note: "Folders too" });
  assert.equal(response.code, 200);
  assert.ok(issues[0].body.startsWith(owner));
  assert.equal(issues[0].body.match(/^- \[ \] /gm).length, 1);
  assert.ok(issues[0].body.includes("- [ ] `Sort   @\u200bwork by date` (+2)"));
  assert.ok(issues[0].body.includes("  Note: `Oldest first`"));
  assert.ok(issues[0].body.includes("  Note: `Folders too`"));
});

test("creates a fresh task for a ticked duplicate, then increments the fresh task", async () => {
  const ticked = "- [X] `Sort files` (+3)";
  const { submit, issues } = harness({ issues: [openIssue(ticked)] });
  await submit({ command: "Sort files" });
  assert.ok(issues[0].body.includes("- [ ] `Sort files` (+1)"));
  await submit({ command: "sort FILES" });
  assert.ok(issues[0].body.startsWith(ticked));
  assert.ok(issues[0].body.includes("- [ ] `Sort files` (+2)"));
});

test("preserves an owner-written Markdown example that looks like a request", async () => {
  const owner = "Example:\n\n```markdown\n- [ ] `Sort files` (+1)\n```";
  const { submit, issues } = harness({ issues: [openIssue(owner)] });
  await submit({ command: "Sort files" });
  await submit({ command: "Sort files" });
  assert.ok(issues[0].body.startsWith(owner));
  assert.ok(issues[0].body.includes("\n\n- [ ] `Sort files` (+2)"));
});

test("acknowledges a honeypot without contacting GitHub or using the rate limit", async () => {
  const { submit, calls } = harness();
  for (let i = 0; i < 6; i++) {
    const response = await submit({ command: "", website: "" });
    assert.equal(response.code, 200);
    assert.deepEqual(response.data, { ok: true });
  }
  assert.equal(calls.length, 0);
  assert.equal((await submit({ command: "Sort files" })).code, 200);
});

test("rejects malformed, oversized, control-character and URL-looking input", async () => {
  const { submit, calls } = harness();
  assert.equal((await submit({}, { method: "GET" })).code, 405);
  assert.equal((await submit({}, { contentType: "text/plain" })).code, 415);
  for (const body of [
    null, [], "{", { command: true }, { command: "   " }, { command: "x".repeat(301) },
    { command: "Sort files", note: "x".repeat(501) }, { command: "Sort files", note: null },
    { command: "\nSort files" }, { command: "Sort files", note: "text\u0000" },
    { command: "Sort files", note: "text\u2028- [ ] fake" },
    { command: "open https://example.com" }, { command: "open www.example.com" },
    { command: "open example.com" }, { command: "open //example.invalid" },
    { command: "Sort files", note: "mailto:person@example.invalid" },
  ]) {
    assert.equal((await submit(body)).code, 400, JSON.stringify(body));
  }
  assert.equal(calls.length, 0);
  assert.equal((await submit({ command: "Sort report.pdf" })).code, 200);
});

test("returns 503 when command requests are unconfigured", async () => {
  const { submit, calls } = harness({ env: {} });
  const response = await submit({ command: "Sort files" });
  assert.equal(response.code, 503);
  assert.match(response.data.error, /aren't set up/);
  assert.equal(calls.length, 0);
});

test("limits each IP to five requests per ten minutes", async () => {
  let time = 1000;
  const { submit, calls } = harness({ now: () => time });
  for (let i = 0; i < 5; i++) assert.equal((await submit({ command: "Sort files" })).code, 200);
  const before = calls.length;
  const limited = await submit({ command: "Sort files" });
  assert.equal(limited.code, 429);
  assert.equal(limited.headers["Retry-After"], "600");
  assert.equal(calls.length, before);
  assert.equal((await submit({ command: "Sort files" }, { ip: "192.0.2.2" })).code, 200);
  time += 10 * 60_000;
  assert.equal((await submit({ command: "Sort files" })).code, 200);
});

test("rolls over to the next issue, links from the old issue, and uses the newest issue", async () => {
  const oldBody = "x".repeat(59_990);
  const { submit, issues } = harness({ issues: [openIssue("Part one"), openIssue(oldBody, 3, "Command requests (2)")] });
  assert.equal((await submit({ command: "Sort files" })).code, 200);
  assert.equal(issues[1].state, "open");
  assert.equal(issues[1].body, `Continued in [#4](https://github.com/ruck-syu/Voxi/issues/4).\n\n${oldBody}`);
  assert.equal(issues[2].title, "Command requests (3)");
  assert.match(issues[2].body, /Sort files/);
  assert.equal((await submit({ command: "Sort files" })).code, 200);
  assert.equal(issues.length, 3);
  assert.ok(issues[2].body.includes("- [ ] `Sort files` (+2)"));
});

test("retries a lost write even when a concurrent writer produced the same count", async () => {
  let patches = 0;
  const { submit, issues } = harness({
    issues: [openIssue("- [ ] `Sort files` (+1)\n- [ ] `Group photos` (+1)")],
    afterPatch(issue) {
      if (++patches === 1) issue.body = "- [ ] `Sort files` (+2)\n- [x] `Group photos` (+1)\n\nOwner added this.";
    },
  });
  assert.equal((await submit({ command: "Sort files", note: "Newest first" })).code, 200);
  assert.equal(patches, 2);
  assert.ok(issues[0].body.includes("- [ ] `Sort files` (+3)\n  Note: `Newest first`"));
  assert.ok(issues[0].body.includes("- [x] `Group photos` (+1)\n\nOwner added this."));
});

test("accepts verification when the owner adds text below the receipt", async () => {
  let patches = 0;
  const { submit, issues } = harness({
    issues: [openIssue("Owner's checklist")],
    afterPatch(issue) {
      if (++patches === 1) issue.body += "\n\nOwner added this below the request.";
    },
  });
  assert.equal((await submit({ command: "Sort files" })).code, 200);
  assert.equal(patches, 1);
  assert.equal((await submit({ command: "Sort files" })).code, 200);
  assert.match(issues[0].body, /- \[ \] `Sort files` \(\+2\)/);
  assert.match(issues[0].body, /Owner added this below the request\./);
  assert.equal(issues[0].body.match(/<!-- voxi-request-receipts:/g).length, 1);
});

test("caps verification retries when updates keep disappearing", async () => {
  let patches = 0;
  const { submit } = harness({
    issues: [openIssue("Owner's checklist")],
    afterPatch(issue, before) { patches++; issue.body = before; },
  });
  assert.equal((await submit({ command: "Sort files" })).code, 502);
  assert.equal(patches, 4);
});

test("queues concurrent requests within the same function instance", async () => {
  const { submit, issues } = harness();
  const results = await Promise.all([submit({ command: "Sort files" }), submit({ command: "sort files" })]);
  assert.ok(results.every(({ code }) => code === 200));
  assert.equal(issues.length, 1);
  assert.ok(issues[0].body.includes("- [ ] `Sort files` (+2)"));
});
