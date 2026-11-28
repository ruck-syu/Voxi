import assert from "node:assert/strict";
import { test } from "node:test";
import pkg from "../package.json" with { type: "json" };
import { sanitizeWrongReport } from "../src/wrong-report.ts";

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
  assert.deepEqual(sanitizeWrongReport("move PDFs to Documents", outcome, "Leave old copies"), {
    command: "move PDFs to Documents",
    outcome: "plan",
    actions: [{ kind: "images_to_pdf", count: 1 }, { kind: "move", count: 32 }],
    note: "Leave old copies",
    appVersion: pkg.version,
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
    assert.deepEqual(sanitizeWrongReport("move PDFs", outcome, ""), {
      command: "move PDFs", outcome: expected, actions: [], note: "", appVersion: pkg.version,
    });
  }
});

test("the public copy rejects path-like text and out-of-bounds fields before sending", () => {
  const outcome = { status: "not_understood" };
  for (const text of [
    "/Users/alex/Secret", "~/Downloads", "C:\\Users\\alex", "../draft", "nested/folder",
    "%2FUsers%2Falex", "／Users／alex", "secrets.txt", ".ssh", "@alex", "line\nbreak",
  ]) {
    assert.ok(sanitizeWrongReport(text, outcome, "").error, text);
    assert.ok(sanitizeWrongReport("sort downloads", outcome, text).error, text);
  }
  assert.ok(sanitizeWrongReport(" ", outcome, "").error);
  assert.ok(sanitizeWrongReport("x".repeat(301), outcome, "").error);
  assert.ok(sanitizeWrongReport("sort downloads", outcome, "x".repeat(501)).error);
});
