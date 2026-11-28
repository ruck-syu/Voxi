import schema from "./_wrong-result-schema.json" with { type: "json" };

const FORBIDDEN = /[\p{M}\p{Default_Ignorable_Code_Point}\p{Cc}\p{Cf}\p{Co}\p{Cn}\p{Zl}\p{Zp}]/u;
const OUTSIDE_ALPHABET = /[^\p{L}\p{N} ',()!?"-]/u;
// Only standalone clock times can contain a colon.
const CLOCK = /(^|[ (),!?"])(?:[01]?\d|2[0-3]):[0-5]\d(?=$|[ (),!?"])/g;
const DRIVE_LETTER = /[a-z]:/i;
// Bare hosts that look like links, without treating filenames like report.pdf,
// Safari.app or logo.ai as links in the separate command-request flow.
export const URL_TEXT = /[a-z][a-z\d+.-]*:\/\/|\b(?:https?|ftp|file|mailto|tel|data|javascript):|\bwww\.|\b[\w-]+\.(?:com|org|net|edu|gov|io|dev|info|biz|co|uk|me|xyz|test|example|local)\b|(?:^|[\s(<])\/\/[\w\d-]+|\b(?:\d{1,3}\.){3}\d{1,3}(?=[:/])/i;

/** Check the normalized text without rewriting the public copy. */
export function wrongResultTextError(text, field) {
  const normalized = text.normalize("NFKC");
  const max = field === "command" ? schema.maxCommand : schema.maxNote;
  if (text.length > max || normalized.length > max) {
    return `Use up to ${max} characters for the ${field}.`;
  }
  if (FORBIDDEN.test(normalized) || OUTSIDE_ALPHABET.test(normalized.replace(CLOCK, "$1")) ||
      URL_TEXT.test(normalized) || DRIVE_LETTER.test(normalized)) {
    return `Remove file names, paths, links, and unsupported characters from the ${field}.`;
  }
  if (field === "command" && !normalized.trim()) return "Enter a command before sending.";
}
