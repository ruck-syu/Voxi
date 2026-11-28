import schema from "../../website/api/_wrong-result-schema.mjs";
import { validateWrongResultText } from "../../website/api/_wrong-result-text.mjs";

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

export type WrongReportFieldErrors = Partial<Record<"command" | "note", string>>;

export const { maxCommand, maxNote } = schema;

/** Project the outcome onto a fixed list of identifiers and integer counts. */
export function reportFields(outcome: ReportOutcome): Pick<WrongResultReport, "outcome" | "actions"> {
  const status = schema.outcomes.includes(outcome.status) ? outcome.status : "problem";
  const counts = status === "plan" || status === "done" ? outcome.actionCounts : undefined;
  const actions = schema.actionKinds.flatMap((kind) => {
    const count = counts?.[kind];
    return typeof count === "number" && Number.isSafeInteger(count) && count > 0 ? [{ kind, count }] : [];
  });
  return { outcome: status, actions };
}

/** User text can cross the network only after the public copy passes this check. */
export function sanitizeWrongReport(command: string, outcome: ReportOutcome, note: string, appVersion: string): WrongResultReport | { error: string; fields: WrongReportFieldErrors } {
  const checkedCommand = validateWrongResultText(command, "command");
  const checkedNote = validateWrongResultText(note, "note");
  const fields = {
    command: "error" in checkedCommand ? checkedCommand.error : undefined,
    note: "error" in checkedNote ? checkedNote.error : undefined,
  };
  if ("error" in checkedCommand) return { error: checkedCommand.error, fields };
  if ("error" in checkedNote) return { error: checkedNote.error, fields };
  return { command: checkedCommand.value, ...reportFields(outcome), note: checkedNote.value, appVersion };
}
