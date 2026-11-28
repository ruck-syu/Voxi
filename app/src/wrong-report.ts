import schema from "../../website/api/_wrong-result-schema.json" with { type: "json" };
import pkg from "../package.json" with { type: "json" };

const { version } = pkg;

export type ReportedActions = Readonly<Record<string, number>>;
export type ReportOutcome = {
  status: string;
  actionCounts?: ReportedActions;
};
export type WrongResultReport = {
  command: string;
  outcome: string;
  actions: { kind: string; count: number }[];
  note: string;
  appVersion: string;
};

const privateText = new RegExp(schema.privateTextPattern, "iu");
const control = /[\p{Cc}\p{Cf}\p{Zl}\p{Zp}]/u;
export const { maxCommand, maxNote } = schema;

/** Project the outcome onto a fixed list of identifiers and integer counts. */
export function reportFields(outcome: ReportOutcome): Pick<WrongResultReport, "outcome" | "actions" | "appVersion"> {
  const status = schema.outcomes.includes(outcome.status) ? outcome.status : "problem";
  const counts = status === "plan" || status === "done" ? outcome.actionCounts : undefined;
  const actions = schema.actionKinds.flatMap((kind) => {
    const count = counts?.[kind];
    return typeof count === "number" && Number.isSafeInteger(count) && count > 0 ? [{ kind, count }] : [];
  });
  return { outcome: status, actions, appVersion: version };
}

/** User text can cross the network only after the public copy passes this check. */
export function sanitizeWrongReport(command: string, outcome: ReportOutcome, note: string): WrongResultReport | { error: string } {
  if (!command.trim() || command.length > maxCommand || note.length > maxNote) {
    return { error: `Use 1–${maxCommand} characters for the command and up to ${maxNote} for the note.` };
  }
  if ([command, note].some((text) => privateText.test(text.normalize("NFKC")) || control.test(text))) {
    return { error: "Remove paths, filenames, usernames and line breaks from the command and note." };
  }
  return { command, ...reportFields(outcome), note };
}
