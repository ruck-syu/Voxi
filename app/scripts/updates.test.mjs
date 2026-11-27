import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";
import ts from "typescript";

const source = readFileSync(new URL("../src/updates.ts", import.meta.url), "utf8");
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
});

function loadUpdates({ refreshVocabulary, check, relaunch, invoke = async () => {} }) {
  const modules = {
    "./api": { api: { refreshVocabulary }, errorMessage: String, inTauri: true },
    "react": { useSyncExternalStore: (_, snapshot) => snapshot() },
    "@tauri-apps/api/core": { invoke },
    "@tauri-apps/api/app": {},
    "@tauri-apps/plugin-updater": { check },
    "@tauri-apps/plugin-process": { relaunch },
  };
  const exports = {};
  runInNewContext(outputText, { exports, require: (name) => modules[name] });
  return exports;
}

const flushUpdates = () => new Promise((resolve) => setImmediate(resolve));

test("the app check finishes while the vocabulary refresh is pending", async () => {
  const vocabulary = Promise.withResolvers();
  const app = Promise.withResolvers();
  const calls = [];
  const updates = loadUpdates({
    refreshVocabulary: () => { calls.push("vocabulary"); return vocabulary.promise; },
    check: () => { calls.push("app"); return app.promise; },
  });
  const check = updates.checkAndInstall();
  await flushUpdates();
  assert.deepEqual(calls, ["vocabulary", "app"]);
  assert.equal(updates.canCheckForUpdates(updates.useUpdateState()), false);

  app.resolve(null);
  await check;
  assert.equal(updates.useUpdateState().k, "latest");
  assert.equal(updates.canCheckForUpdates(updates.useUpdateState()), true);
  assert.equal(updates.useUpdateState().commandsAdded, false);
  vocabulary.resolve("updated");
  await flushUpdates();
  assert.equal(updates.useUpdateState().k, "latest");
  assert.equal(updates.useUpdateState().commandsAdded, true);
});

test("a late vocabulary result cannot add a notice to the next check", async () => {
  const vocabulary = Promise.withResolvers();
  const refreshes = [vocabulary.promise, Promise.resolve("unchanged")];
  const updates = loadUpdates({
    refreshVocabulary: () => refreshes.shift(),
    check: async () => null,
  });
  await updates.checkAndInstall();
  await updates.checkAndInstall();
  vocabulary.resolve("updated");
  await flushUpdates();
  assert.equal(updates.useUpdateState().k, "latest");
  assert.equal(updates.useUpdateState().commandsAdded, false);
});

test("vocabulary rejection leaves app installation and relaunch failure handling intact", async () => {
  const calls = [];
  const updates = loadUpdates({
    refreshVocabulary: async () => { throw new Error("Vocabulary unavailable"); },
    check: async () => ({
      version: "0.2.0",
      downloadAndInstall: async () => { calls.push("install"); },
    }),
    relaunch: async () => { calls.push("relaunch"); throw new Error("Relaunch unavailable"); },
    invoke: async (command) => { calls.push(command); },
  });
  await updates.checkAndInstall();
  assert.deepEqual(calls, ["install", "relaunch", "updater_failed"]);
  assert.equal(updates.useUpdateState().k, "relaunchFailed");
  assert.equal(updates.useUpdateState().commandsAdded, false);
  assert.equal(updates.canCheckForUpdates(updates.useUpdateState()), false);
});
