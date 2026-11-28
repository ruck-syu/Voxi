import schema from "./_wrong-result-schema.mjs";

const FORBIDDEN = /[\p{M}\p{Default_Ignorable_Code_Point}\p{Cc}\p{Cf}\p{Co}\p{Cn}\p{Cs}\p{Zl}\p{Zp}]/u;
const OUTSIDE_ALPHABET = /[^\p{L}\p{N} ',()!?"‘’“”–—&+-]/u;
// Only one period, at the very end (dictation adds it). Periods anywhere else
// can rebuild names, versions and hosts ("report. pdf", "Figma-124.3", "evil. de").
const FINAL_PERIOD = /([\p{L}\p{N})'"’”])\.$/u;
// Only standalone clock times can contain a colon.
const CLOCK = /(^|[ (),!?"‘’“”–—-])(?:[01]?\d|2[0-3]):[0-5]\d(?=$|[ (),!?"‘’“”–—-])/g;
const DRIVE_LETTER = /(?:^|[^\p{L}])[a-z]:(?=.)/iu;
// Bare hosts that look like links, without treating filenames like report.pdf,
// Safari.app or logo.ai as links in the separate command-request flow.
export const URL_TEXT = /[a-z][a-z\d+.-]*:\/\/|\b(?:https?|ftp|file|mailto|tel|data|javascript):|\bwww\.|\b[\w-]+\.(?:com|org|net|edu|gov|io|dev|info|biz|co|uk|me|xyz|test|example|local)\b|(?:^|[\s(<])\/\/[\w\d-]+|\b(?:\d{1,3}\.){3}\d{1,3}(?=[:/])/i;

function collapseWhitespace(text) {
  return text.replace(/\s+/gu, " ").trim();
}

export function canonicalWrongResultText(text) {
  return collapseWhitespace(text.normalize("NFKC"));
}

function unsupportedCharacter(field, character) {
  if (/[\n\r\u2028\u2029]/u.test(character)) return { error: `Remove line breaks from the ${field}.` };
  if (character === "\t") return { error: `Remove tabs from the ${field}.` };
  const name = FORBIDDEN.test(character)
    ? `U+${character.codePointAt(0).toString(16).toUpperCase().padStart(4, "0")}`
    : `“${character}”`;
  return { error: `Remove the unsupported character ${name} from the ${field}.` };
}

/** Return the exact public copy after checking it. */
export function validateWrongResultText(text, field) {
  const normalized = text.normalize("NFKC");
  // Check before collapsing whitespace so line breaks, tabs and hidden characters still fail.
  const forbidden = normalized.match(FORBIDDEN);
  if (forbidden) return unsupportedCharacter(field, forbidden[0]);

  const value = collapseWhitespace(normalized);
  const max = field === "command" ? schema.maxCommand : schema.maxNote;
  if (value.length > max) return { error: `Use up to ${max} characters for the ${field}.` };
  if (field === "command" && !value) return { error: "Enter a command before sending." };

  const unchecked = value.replace(FINAL_PERIOD, "$1").replace(CLOCK, "$1");
  // Examine the original dots, including hosts split by spaces after a period.
  const possibleHost = value.replace(/\. +/g, ".");
  if (URL_TEXT.test(possibleHost) || /(?:\p{Nd}{1,3}\.){3}\p{Nd}{1,3}/u.test(possibleHost)) {
    return { error: `Remove links or host names from the ${field}.` };
  }
  if (DRIVE_LETTER.test(unchecked) || /[/\\]/.test(unchecked)) {
    return { error: `Remove paths from the ${field}.` };
  }
  if (unchecked.includes(".")) return { error: `Remove file names, extensions and periods before the end from the ${field}.` };
  const unsupported = unchecked.match(OUTSIDE_ALPHABET);
  if (unsupported) return unsupportedCharacter(field, unsupported[0]);
  return { value };
}
