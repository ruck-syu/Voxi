import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import textCases from "../../.github/scripts/fixtures/wrong-result-text.mjs";
import { sanitizeWrongReport } from "../src/wrong-report.ts";

const { version: appVersion } = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8"));
const sanitize = (command, outcome, note) => sanitizeWrongReport(command, outcome, note, appVersion);

test("the public report keeps only the typed text and allowlisted plan counts", () => {
  const outcome = {
    status: "plan",
    id: 7,
    title: "Move report.pdf into /Users/alex/Documents",
    warnings: ["~/Private/annual-report.pdf already exists"],
    items: [{ kind: "move", primary: "report.pdf", detail: "/Users/alex/Documents", note: "annual-report.pdf" }],
    actionCounts: { move: 32, images_to_pdf: 1, "/Users/alex/Secret": 8, "invoice.pdf": 3, zip: "private.zip", unzip: -1 },
    searchStats: { visited: 4, lastPath: "/Users/alex/Private" },
  };
  assert.deepEqual(sanitize("move PDFs to Documents", outcome, "Leave old copies"), {
    command: "move PDFs to Documents",
    outcome: "plan",
    actions: [{ kind: "images_to_pdf", count: 1 }, { kind: "move", count: 32 }],
    note: "Leave old copies",
    appVersion,
  });
});

test("problem and info messages, result summaries and forged statuses cannot leak", () => {
  for (const outcome of [
    { status: "info", rows: [{ name: "passwords.txt" }], actionCounts: { move: 3 } },
    { status: "problem", message: "Missing /Users/alex/Secrets" },
    { status: "done", summary: "Moved secret.png", error: "~/Photos" },
    { status: "/Users/alex" },
  ]) {
    const expected = outcome.status.startsWith("/") ? "problem" : outcome.status;
    assert.deepEqual(sanitize("move PDFs", outcome, ""), {
      command: "move PDFs", outcome: expected, actions: [], note: "", appVersion,
    });
  }
});

test("the public copy rejects path-like text and out-of-bounds fields before sending", () => {
  const outcome = { status: "not_understood" };
  for (const text of [
    "/Users/alex/Secret", "~/Downloads", "C:\\Users\\alex", "../draft", "nested/folder",
    "%2FUsers%2Falex", "／Users／alex", "secrets.txt", ".ssh", "@alex", "line\nbreak",
  ]) {
    assert.ok(sanitize(text, outcome, "").error, text);
    assert.ok(sanitize("sort downloads", outcome, text).error, text);
  }
  assert.ok(sanitize(" ", outcome, "").error);
  assert.ok(sanitize("x".repeat(301), outcome, "").error);
  assert.ok(sanitize("sort downloads", outcome, "x".repeat(501)).error);
});

test("the public copy rejects unsafe text in either field before sending", () => {
  const outcome = { status: "plan" };
  for (const text of textCases.rejected) {
    const result = sanitize(text, outcome, text);
    assert.match(result.error, /^Remove .+ from the command\.$/, JSON.stringify(text));
    assert.equal(result.fields.command, result.error);
    assert.match(result.fields.note, /^Remove .+ from the note\.$/, JSON.stringify(text));
    assert.equal(sanitize("sort Downloads", outcome, text).error, result.fields.note);
  }
});

test("the public copy uses exactly the validated canonical text", () => {
  const accepted = [...textCases.accepted.map((text) => [text, text]), ...textCases.canonical];
  for (const [text, expected] of accepted) {
    const result = sanitize(text, { status: "plan" }, text);
    assert.equal(result.command, expected);
    assert.equal(result.note, expected);
  }
  for (const note of ["    ", "\u00a0"]) {
    assert.equal(sanitize("move PDFs", { status: "plan" }, note).note, "");
  }
});

test("the public copy describes what to remove from each field", () => {
  const outcome = { status: "plan" };
  const result = sanitize("report.pdf", outcome, "a/b");
  assert.deepEqual(result.fields, {
    command: "Remove file names or extensions from the command.",
    note: "Remove paths from the note.",
  });
  assert.equal(sanitize("evil. com", outcome, "").error, "Remove links or host names from the command.");
  assert.equal(sanitize("move PDFs", outcome, "@alex").error, "Remove the unsupported character “@” from the note.");
  assert.equal(sanitize("move PDFs", outcome, "file;name").error, "Remove the unsupported character “;” from the note.");
  assert.equal(sanitize("move PDFs", outcome, "Note: leave copies").error, "Remove the unsupported character “:” from the note.");
});
