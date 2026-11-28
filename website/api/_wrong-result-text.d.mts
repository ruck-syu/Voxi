export const URL_TEXT: RegExp;

export function canonicalWrongResultText(text: string): string;
export function validateWrongResultText(text: string, field: "command" | "note"): { value: string } | { error: string };
