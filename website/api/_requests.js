// Shared by the endpoint (which only queues requests as issue comments) and
// the GitHub Action that folds them into the checklist. Vercel does not turn
// files starting with "_" into functions.

const LABEL = "command-requests";
const TITLE = "Command requests";
const HEADER = "Requests submitted from Voxi. Tick a command when Voxi supports it.";
const MARKER = "<!-- voxi-command-request:";
const BODY_LIMIT = 60_000;
const MAX_COMMAND = 300;
const MAX_NOTE = 500;
const KEEP_RECEIPTS = 200;
const RECEIPT_LINE = /^<!-- voxi-folded: ([\d,]*) -->\r?$/;
// Cf covers bidi overrides and zero-width characters that could disguise text.
const CONTROL = /[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/u;
const CONTROL_GLOBAL = new RegExp(CONTROL.source, "gu");
// Bare hosts that look like links, without treating filenames like report.pdf,
// Safari.app or logo.ai as links. Inline code already stops autolinking.
const URL = /[a-z][a-z\d+.-]*:\/\/|\b(?:https?|ftp|file|mailto|tel|data|javascript):|\bwww\.|\b[\w-]+\.(?:com|org|net|edu|gov|io|dev|info|biz|co|uk|me|xyz|test|example|local)\b|(?:^|[\s(<])\/\/[\w\d-]+|\b(?:\d{1,3}\.){3}\d{1,3}(?=[:/])/i;
const TASK = /^([-*+] \[ \] )(`+)(.*?)\2 \(\+([1-9]\d*)\)([ \t]*\r?)$/;

/** The trimmed command and note, or a user-facing error. */
function validate(rawCommand, rawNote = "") {
  if (typeof rawCommand !== "string" || typeof rawNote !== "string") {
    return { error: "Send the command and note as text." };
  }
  const command = rawCommand.trim();
  const note = rawNote.trim();
  if (!command || command.length > MAX_COMMAND || note.length > MAX_NOTE) {
    return { error: "Use 1–300 characters for the command and up to 500 for the note." };
  }
  if ([rawCommand, rawNote].some((text) => CONTROL.test(text) || URL.test(text))) {
    return { error: "Remove links and line breaks from the command and note." };
  }
  return { command, note };
}

/** A queued request. The payload is base64 so no text can close the HTML comment. */
function encodeComment(command, note) {
  const payload = Buffer.from(JSON.stringify({ command, note }), "utf8").toString("base64");
  return `${MARKER}${payload} -->\nQueued for the checklist.`;
}

/** The request in a queued comment, validated again, or null. */
function decodeComment(body) {
  if (typeof body !== "string" || !body.startsWith(MARKER)) return null;
  const end = body.indexOf(" -->", MARKER.length);
  if (end === -1) return null;
  try {
    const { command, note = "" } = JSON.parse(Buffer.from(body.slice(MARKER.length, end), "base64").toString("utf8"));
    const request = validate(command, note);
    return request.error ? null : request;
  } catch {
    return null;
  }
}

function normalized(text) {
  return text.trim().replace(/\s+/gu, " ").toLowerCase();
}

function inlineCode(text) {
  text = text.replace(CONTROL_GLOBAL, "").replaceAll("@", "@​");
  const longestRun = Math.max(0, ...(text.match(/`+/g) ?? []).map((run) => run.length));
  const fence = "`".repeat(longestRun + 1);
  const content = text.startsWith("`") || text.endsWith("`") ? ` ${text} ` : text;
  return `${fence}${content}${fence}`;
}

/** Comment ids already folded into this body. */
function receipts(body) {
  return body.split("\n").flatMap((line) => line.match(RECEIPT_LINE)?.[1].split(",").filter(Boolean) ?? []);
}

function findOpenTask(lines, key) {
  let codeFence = "";
  return lines.findIndex((line) => {
    const fence = line.match(/^ {0,3}(`{3,}|~{3,})(.*)$/);
    if (codeFence) {
      if (fence && fence[1].startsWith(codeFence) && !fence[2].trim()) codeFence = "";
      return false;
    }
    if (fence && (fence[1][0] === "~" || !fence[2].includes("`"))) {
      codeFence = fence[1];
      return false;
    }
    const task = line.match(TASK);
    return task && normalized(task[3].replaceAll("@​", "@")) === key;
  });
}

/**
 * Adds one request to a checklist body. A repeat of an unticked command bumps
 * its count; a repeat of a ticked one adds a fresh line. Everything else the
 * owner wrote is kept as is.
 */
function fold(body, { command, note }, id) {
  const lines = body.split("\n").filter((line) => !RECEIPT_LINE.test(line));
  const index = findOpenTask(lines, normalized(command));
  const noteLine = note ? [`  Note: ${inlineCode(note)}`] : [];
  if (index === -1) {
    lines.push("", `- [ ] ${inlineCode(command)} (+1)`, ...noteLine);
  } else {
    const [, prefix, fence, text, count, trailing] = lines[index].match(TASK);
    lines.splice(index, 1, `${prefix}${fence}${text}${fence} (+${BigInt(count) + 1n})${trailing}`, ...noteLine);
  }
  const kept = [...receipts(body), id].slice(-KEEP_RECEIPTS);
  return `${lines.join("\n").replace(/\n+$/, "")}\n<!-- voxi-folded: ${kept.join(",")} -->`;
}

module.exports = {
  LABEL,
  TITLE,
  HEADER,
  MARKER,
  BODY_LIMIT,
  validate,
  encodeComment,
  decodeComment,
  inlineCode,
  receipts,
  fold,
};
