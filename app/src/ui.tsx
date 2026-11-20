import type { Shortcut } from "./api";

/** Mirrors parse::MAX_COMMAND_BYTES (src-tauri/src/parse/mod.rs): the backend
 *  refuses raw UTF-8 input over this size before trimming or normalizing. */
export const MAX_COMMAND_BYTES = 16_384;
/** Mirrors settings::check_shortcut_commands: at most eight commands, and one
 *  shared byte budget for the whole expansion. */
export const MAX_SHORTCUT_COMMANDS = 8;
export const TOO_LONG = "That command is too long.";
/** UTF-8 byte length, not code points: a dictated emoji can be four bytes. */
export const utf8Bytes = (text: string): number => new TextEncoder().encode(text).length;

/** Which parts of a saved shortcut the backend would refuse for length. Raw
 *  values are checked (before trimming), matching check_shortcut_commands. */
export function shortcutLengthIssue(s: Shortcut): { phrase: boolean; commands: boolean; any: boolean } {
  const phrase = utf8Bytes(s.phrase) > MAX_COMMAND_BYTES;
  const total = s.commands.reduce((n, c) => n + utf8Bytes(c), 0);
  const commands = s.commands.length > MAX_SHORTCUT_COMMANDS || total > MAX_COMMAND_BYTES;
  return { phrase, commands, any: phrase || commands };
}

export function prettyHotkey(h: string): string {
  const map: Record<string, string> = {
    control: "⌃", ctrl: "⌃", alt: "⌥", option: "⌥", shift: "⇧", super: "⌘", cmd: "⌘", command: "⌘", commandorcontrol: "⌘",
  };
  return h
    .split("+")
    .map((p) => map[p.toLowerCase()] ?? p)
    .join("");
}

/** Row index for the staggered "inked onto the card" entrance. */
export const row = (i: number) => ({ "--i": Math.min(i, 12) }) as React.CSSProperties;

export function Sheet(props: {
  title: string;
  aside?: React.ReactNode;
  bodyClass?: string;
  foot: React.ReactNode;
  footRight?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <>
      <div className="head sheet-head">
        <span className="rule" aria-hidden="true" />
        <h1 className="sheet-title">{props.title}</h1>
        {props.aside}
      </div>
      <section className={props.bodyClass ? `body sheet-body ${props.bodyClass}` : "body sheet-body"}>{props.children}</section>
      <footer className="foot">
        <span>{props.foot}</span>
        {props.footRight}
      </footer>
    </>
  );
}
