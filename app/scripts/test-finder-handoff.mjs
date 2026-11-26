import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

const app = readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");
const effect = app.match(/const scopeGeneration = [^\n]+\n  useEffect\(\(\) => \{([\s\S]*?)\n  \}, \[showRequestId\]\);/);
assert.ok(effect, "the command view reads the opening context when its show request changes");

for (const order of [[1, 2], [2, 1]]) {
  test(`only the current opening populates the list when replies arrive in order ${order.join(", ")}`, async () => {
    const calls = [];
    const replies = new Map();
    const scopes = [];
    const notices = [];
    const scopeGeneration = { current: null };
    const run = (showRequestId) => runInNewContext(`(() => {${effect[1]}})()`, {
      inTauri: true,
      showRequestId,
      api: {
        openingContext: (id) => {
          calls.push(id);
          const reply = Promise.withResolvers();
          replies.set(id, reply.resolve);
          return reply.promise;
        },
      },
      scopeGeneration,
      setResultScope: (scope) => scopes.push(scope),
      setSelectionNotice: (notice) => notices.push(notice),
    });
    run(0);
    assert.deepEqual(calls, [], "the initial hidden view cannot claim a Finder selection");
    run(1)();
    run(2);
    assert.deepEqual(calls, [1, 2]);

    const latest = { label: "your Finder selection", count: 2 };
    for (const id of order) {
      replies.get(id)(id === 2
        ? { generation: 2, scope: latest, notice: "One item was left out." }
        : { generation: 1, scope: null, notice: null });
      await Promise.resolve();
    }
    assert.deepEqual(scopes, [latest]);
    assert.deepEqual(notices, ["One item was left out."]);
    assert.equal(scopeGeneration.current, 2);
  });
}

test("finishing a run refreshes the Finder list for the window that reopened during the run", async () => {
  const refresh = app.match(/(const result = await api\.confirm[\s\S]*?)\n      setPhase\(\{ k: "done"/);
  assert.ok(refresh);
  const confirm = Promise.withResolvers();
  const latestShowId = { current: 1 };
  const scopeGeneration = { current: 3 };
  const calls = [];
  const scopes = [];
  const done = runInNewContext(`(async () => {${refresh[1]}})()`, {
    plan: { id: 1 },
    skipped: [],
    showRequestId: 1,
    latestShowId,
    scopeGeneration,
    api: {
      confirm: () => confirm.promise,
      openingContext: async (id) => {
        calls.push(id);
        return id === 2 ? { generation: 3, scope: { count: 0 }, notice: null } : null;
      },
    },
    setResultScope: (scope) => scopes.push(scope),
    setSelectionNotice: () => {},
  });
  latestShowId.current = 2;
  confirm.resolve({});
  await done;
  assert.deepEqual(calls, [2]);
  assert.deepEqual(scopes, [{ count: 0 }]);
});

test("a cancelled plan leaves the view's current scope and generation alone", async () => {
  const update = app.match(/(const \{ outcome, scope, generation, search_stats:[\s\S]*?)\n        setPhase\(\{ k: "outcome"/);
  assert.ok(update);
  const scopeGeneration = { current: 1 };
  const scopes = [];
  await runInNewContext(`(async () => {${update[1]}})()`, {
    command: "find PDFs",
    picked: {},
    request: 1,
    requestId: { current: 1 },
    undoEntryId: undefined,
    replaces: undefined,
    api: { plan: async () => ({ outcome: { status: "problem" }, scope: null, generation: null }) },
    scopeGeneration,
    setResultScope: (scope) => scopes.push(scope),
  });
  assert.equal(scopeGeneration.current, 1);
  assert.deepEqual(scopes, []);
});

test("a normal show clears the setup notice and ignores a late welcome reply", async () => {
  const shown = app.match(/"voxi:\/\/shown", \(e\) => \{([\s\S]*?)\n    \}\);/);
  assert.ok(shown);
  const opening = Promise.withResolvers();
  const calls = [];
  let notice = "Finish setting up Voxi";
  const show = runInNewContext(`(e) => {${shown[1]}}`, {
    api: { openingContext: (id) => { calls.push(id); return opening.promise; } },
    cancelClose: () => {},
    showId: { current: 0 },
    setShowRequestId: () => {},
    setSetupNotice: (value) => { notice = value; },
    disposed: false,
    awaitingShow: {},
    executingRef: { current: false },
    auxiliaryOperations: { current: new Set() },
    setCommandSession: () => {},
    setView: (update) => assert.equal(update("welcome"), "welcome"),
    shownAt: {},
    performance,
    setOpen: () => {},
    setPendingShow: () => {},
  });
  show({ payload: { requestId: 1, mode: "welcome" } });
  notice = "Finish setting up Voxi";
  show({ payload: { requestId: 2, mode: "command" } });
  assert.equal(notice, null);
  opening.resolve({ notice: "Stale Finder notice" });
  await Promise.resolve();
  assert.equal(notice, null);
  assert.deepEqual(calls, [1]);
});
