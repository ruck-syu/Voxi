// Folds queued command-request comments into the tracking issue's checklist.
// The workflow runs this one at a time (a concurrency group), and every run
// folds everything still queued, so a cancelled pending run loses nothing.
// Each comment id is recorded in the body before the comment is deleted, so
// a retry after a partial failure never counts a request twice.
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { LABEL, TITLE, HEADER, MARKER, BODY_LIMIT, decodeComment, receipts, fold } = require("../../website/api/_requests.js");

const MAX_PER_RUN = 100;
const MAX_PARTS = 20;

export async function run({ github, author, repo }) {
  await ensureLabel(github);
  const issues = await openIssues(github);
  if (!issues.length) issues.push(await createIssue(github, TITLE));
  // An older full issue that never got its link (a run failed mid-rollover).
  if (issues.length > 1) await ensureLink(github, repo, issues.at(-2).number, issues.at(-1).number);

  const queued = [];
  const login = author.toLowerCase();
  for (const issue of issues) {
    for (let page = 1; ; page++) {
      const comments = await github(`/issues/${issue.number}/comments?per_page=100&page=${page}`);
      for (const comment of comments) {
        if (comment.user?.login?.toLowerCase() === login && comment.body?.startsWith(MARKER)) queued.push(comment);
      }
      if (comments.length < 100) break;
    }
  }
  queued.sort((a, b) => (BigInt(a.id) === BigInt(b.id) ? 0 : BigInt(a.id) < BigInt(b.id) ? -1 : 1));
  const batch = queued.slice(0, MAX_PER_RUN).map((comment) => ({ id: String(comment.id), request: decodeComment(comment.body) }));

  // Folded ids from every open tracking issue: after a rollover, a request may
  // already be in an older issue while its comment wasn't deleted yet.
  const done = new Set(issues.flatMap((issue) => receipts(issue.body ?? "")));
  const settled = batch.filter(({ id, request }) => !request || done.has(id));
  for (const { id } of settled) await deleteComment(github, id);

  let target = issues.at(-1);
  let fresh = false;
  let pending = batch.filter(({ id, request }) => request && !done.has(id));
  while (pending.length) {
    const left = await save(github, target.number, pending);
    // Delete as soon as an issue is verified, before the next one is opened.
    for (const { id } of pending.slice(0, pending.length - left.length)) await deleteComment(github, id);
    if (!left.length) break;
    if (fresh && left.length === pending.length) throw new Error("A request does not fit in a new issue");
    target = await rollover(github, repo, target);
    fresh = true;
    pending = left;
  }
  const dropped = batch.filter(({ request }) => !request).length;
  return { folded: batch.length - dropped, dropped };
}

/** Folds what fits into one issue; returns the requests that didn't fit. */
async function save(github, number, requests) {
  // Read just before writing: GitHub has no conditional update, so this keeps
  // the window for overwriting an owner's tick as short as possible.
  const issue = await github(`/issues/${number}`);
  const original = issue.body ?? "";
  let body = original;
  let index = 0;
  for (; index < requests.length; index++) {
    const { id, request } = requests[index];
    const next = fold(body, request, id);
    if (next.length > BODY_LIMIT) break;
    body = next;
  }
  if (body !== original) {
    await github(`/issues/${number}`, "PATCH", { body });
    const verified = new Set(receipts((await github(`/issues/${number}`)).body ?? ""));
    if (requests.slice(0, index).some(({ id }) => !verified.has(id))) {
      throw new Error(`Could not verify the update to #${number}`);
    }
  }
  return requests.slice(index);
}

async function deleteComment(github, id) {
  await github(`/issues/comments/${id}`, "DELETE", undefined, 404);
}

async function createIssue(github, title) {
  const issue = await github("/issues", "POST", { title, labels: [LABEL], body: HEADER });
  // Only collaborators (and the request token) can comment on a locked issue.
  await github(`/issues/${issue.number}/lock`, "PUT", {});
  return issue;
}

async function rollover(github, repo, from) {
  const part = Number(from.title.match(/^Command requests \((\d+)\)$/)?.[1] ?? 1) + 1;
  if (part > MAX_PARTS) throw new Error(`Refusing to open more than ${MAX_PARTS} tracking issues`);
  const next = await createIssue(github, `${TITLE} (${part})`);
  await ensureLink(github, repo, from.number, next.number);
  return next;
}

async function ensureLink(github, repo, from, to) {
  const link = `Continued in [#${to}](https://github.com/${repo}/issues/${to}).`;
  const old = await github(`/issues/${from}`);
  if (!(old.body ?? "").split(/\r?\n/).includes(link)) {
    await github(`/issues/${from}`, "PATCH", { body: `${link}\n\n${old.body ?? ""}` });
  }
}

async function openIssues(github) {
  const issues = [];
  for (let page = 1; ; page++) {
    const items = await github(`/issues?state=open&labels=${LABEL}&sort=created&direction=asc&per_page=100&page=${page}`);
    issues.push(...items.filter((item) => !item.pull_request));
    if (items.length < 100) break;
  }
  return issues;
}

async function ensureLabel(github) {
  if (await github(`/labels/${LABEL}`, "GET", undefined, 404)) return;
  await github("/labels", "POST", { name: LABEL, color: "1d76db", description: "Commands requested from Voxi" }, 422);
}

export function gitHubClient({ token, repo, fetch = globalThis.fetch }) {
  return async function github(path, method = "GET", data, allowedStatus) {
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
    });
    if (response.status === allowedStatus) return null;
    if (!response.ok) throw new Error(`GitHub returned ${response.status} for ${method} ${path}`);
    return response.status === 204 ? null : response.json();
  };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const { GITHUB_TOKEN: token, GITHUB_REPOSITORY: repo, REQUEST_AUTHOR: author } = process.env;
  if (!token || !repo || !author) throw new Error("GITHUB_TOKEN, GITHUB_REPOSITORY and REQUEST_AUTHOR are required");
  const result = await run({ github: gitHubClient({ token, repo }), author, repo });
  console.log(`Folded ${result.folded} request(s), dropped ${result.dropped} invalid one(s).`);
}
