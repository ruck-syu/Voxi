// Reads a typed or dictated answer to "which one?": "2", "two", "the second
// one", "number 3", "last", or words from the name or folder ("the setup one",
// "the one in Documents").

const CARDINALS: Record<string, number> = {
  two: 2, three: 3, four: 4, five: 5, six: 6, seven: 7, eight: 8, nine: 9, ten: 10,
  to: 2, too: 2, for: 4, ate: 8, // common dictation homophones
};
const ORDINALS: Record<string, number> = {
  first: 1, second: 2, third: 3, fourth: 4, fifth: 5, sixth: 6, seventh: 7, eighth: 8, ninth: 9, tenth: 10,
};
const FILLER = new Set(["the", "a", "one", "number", "option", "item", "file", "folder", "please", "in", "from", "on", "that", "this", "its", "it's", "is", "pick", "choose", "use", "take"]);
const CANCEL = /^(cancel|never ?mind|stop|none|neither|no|forget it)$/;

const loose = (s: string) => s.toLowerCase().replace(/[^\p{L}\p{N}]/gu, "");

export type Choice = { index: number } | { cancel: true } | null;

export function readChoice(answer: string, candidates: string[]): Choice {
  const words = answer
    .toLowerCase()
    .replace(/[.,!?;:"“”]/g, " ")
    .split(/\s+/)
    .filter(Boolean);
  const said = words.join(" ");
  if (!said) return null;
  if (CANCEL.test(said)) return { cancel: true };

  const inRange = (n: number) => (n >= 1 && n <= candidates.length ? { index: n - 1 } : null);
  const meaningful = words.filter((w) => !FILLER.has(w));
  // "one" on its own means 1; inside "the one in Documents" it doesn't.
  if (meaningful.length === 0 && words.includes("one")) return inRange(1);
  for (const w of words) {
    if (w === "last") return inRange(candidates.length);
    const digits = w.match(/^#?(\d+)(st|nd|rd|th)?$/);
    if (digits) return inRange(Number(digits[1]));
    if (w in ORDINALS) return inRange(ORDINALS[w]);
  }
  // Only count homophones like "to" when the answer is nothing else.
  if (meaningful.length === 1 && meaningful[0] in CARDINALS) return inRange(CARDINALS[meaningful[0]]);
  for (const w of meaningful) {
    if (w in CARDINALS && !["to", "too", "for", "ate"].includes(w)) return inRange(CARDINALS[w]);
  }

  // Words from the name or folder that point at exactly one candidate.
  const key = loose(meaningful.join(""));
  if (key.length >= 2) {
    const hits = candidates.map((c, i) => [loose(c), i] as const).filter(([c]) => c.includes(key));
    if (hits.length === 1) return { index: hits[0][1] };
  }
  return null;
}
