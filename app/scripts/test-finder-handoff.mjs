import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

const app = readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8");
const effect = app.match(/const scopeGeneration = [^\n]+\n  useEffect\(\(\) => \{([\s\S]*?)\n  \}, \[showRequestId\]\);/);
assert.ok(effect, "the command view reads the opening context when its show request changes");

test("the hidden view waits for a native show request and a superseded reply cannot replace the list", async () => {
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
        return new Promise((resolve) => replies.set(id, resolve));
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
  replies.get(1)({ generation: 1, scope: null, notice: null });
  replies.get(2)({ generation: 2, scope: latest, notice: "One item was left out." });
  await Promise.resolve();
  assert.deepEqual(scopes, [latest]);
  assert.deepEqual(notices, ["One item was left out."]);
  assert.equal(scopeGeneration.current, 2);
});
