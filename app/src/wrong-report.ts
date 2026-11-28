import schema from "../../website/api/_wrong-result-schema.json" with { type: "json" };
import { wrongResultTextError } from "../../website/api/_wrong-result-text.mjs";
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

export type WrongReportFieldErrors = Partial<Record<"command" | "note", string>>;

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
export function sanitizeWrongReport(command: string, outcome: ReportOutcome, note: string): WrongResultReport | { error: string; fields: WrongReportFieldErrors } {
  const fields = {
    command: wrongResultTextError(command, "command"),
    note: wrongResultTextError(note, "note"),
  };
  const error = fields.command ?? fields.note;
  if (error) return { error, fields };
  return { command, ...reportFields(outcome), note };
}
