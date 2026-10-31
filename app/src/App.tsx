import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { api, inTauri, tildify, type FolderAccess, type Outcome, type RunResult, type Settings, type Shortcut } from "./api";
import { readChoice } from "./choice";
import { appVersion, checkAndInstall, type UpdateState } from "./updates";

type View = "command" | "settings" | "welcome" | "access";
type Phase =
  | { k: "ready" }
  | { k: "parsing" }
  | { k: "outcome"; outcome: Outcome }
  | { k: "running"; title: string }
  | { k: "done"; result: RunResult; wasUndo: boolean }
  | { k: "error"; message: string };

/** Waiting for the answer to "which one?": the command being planned and
 *  the picks made so far (one command can need several). */
type Asking = { command: string; choices: Record<string, string>; name: string; candidates: string[] };

const EXAMPLES = [
  "move screenshots from today to Screenshots",
  "clean up Downloads older than 90 days",
  "convert all HEIC in Pictures to jpg",
];

function prettyHotkey(h: string): string {
  const map: Record<string, string> = {
    control: "⌃", ctrl: "⌃", alt: "⌥", option: "⌥", shift: "⇧", super: "⌘", cmd: "⌘", command: "⌘", commandorcontrol: "⌘",
  };
  return h
    .split("+")
    .map((p) => map[p.toLowerCase()] ?? p)
    .join("");
}

const reducedMotion = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;
const CLOSE_MS = 120;

export default function App() {
  const [view, setView] = useState<View>("command");
  const [settings, setSettings] = useState<Settings | null>(null);
  const [open, setOpen] = useState(true);
  const [height, setHeight] = useState<number | null>(null);
  const [glide, setGlide] = useState(false);
  const innerRef = useRef<HTMLDivElement>(null);

  // Plays the close animation, then hides the window. The card stays in its
  // closed state while hidden, so the next show animates in from there.
  const close = useCallback(() => {
    setOpen(false);
    setTimeout(() => api.hide(), reducedMotion() ? 0 : CLOSE_MS);
  }, []);

  // Clicking outside the window (another app, the desktop, the menu bar)
  // closes it, like Spotlight. Not while a command is being planned or run:
  // a macOS folder-access prompt can take focus mid-command. And not in the
  // moment right after opening, while macOS is still handing over focus.
  const busyRef = useRef(false);
  const setBusy = useCallback((busy: boolean) => {
    busyRef.current = busy;
  }, []);
  const shownAt = useRef(0);
  useEffect(() => {
    if (!inTauri) return;
    const onBlur = () => {
      if (busyRef.current || performance.now() - shownAt.current < 300) return;
      close();
    };
    window.addEventListener("blur", onBlur);
    return () => window.removeEventListener("blur", onBlur);
  }, [close]);

  useEffect(() => {
    api.getSettings().then((s) => {
      setSettings(s);
      if (!s.onboarded) setView("welcome");
    });
    if (!inTauri) return;
    const unShown = listen<string>("voxi://shown", (e) => {
      const mode = e.payload as View;
      setView((v) => (mode === "command" && v === "welcome" ? v : mode));
      shownAt.current = performance.now();
      setOpen(true);
      window.dispatchEvent(new Event("voxi-focus"));
    });
    const unHide = listen("voxi://hide", close);
    return () => {
      unShown.then((f) => f());
      unHide.then((f) => f());
    };
  }, [close]);

  // The window is the frosted card, so it is sized to the content and macOS
  // animates the resize. In a browser preview the card glides instead.
  useLayoutEffect(() => {
    const el = innerRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => {
      const h = Math.ceil(el.offsetHeight);
      if (inTauri) invoke("fit_window", { height: h });
      else setHeight(h);
    });
    ro.observe(el);
    // Only glide after the first measurement, never from zero.
    requestAnimationFrame(() => requestAnimationFrame(() => setGlide(true)));
    return () => ro.disconnect();
  }, []);

  return (
    <div className="card" data-open={open || undefined} data-glide={glide || undefined} style={height ? { height } : undefined}>
      <div className="card-inner" ref={innerRef}>
        {view === "command" && <CommandView
            open={open}
            onClose={close}
            onBusy={setBusy}
            onSettings={() => setView("settings")}
          />}
        {view === "settings" && settings && (
          <SettingsView
            settings={settings}
            onSaved={setSettings}
            onClose={() => setView("command")}
            onAccess={() => setView("access")}
            setBusy={setBusy}
          />
        )}
        {view === "access" && settings && (
          <AccessView settings={settings} firstRun={false} setBusy={setBusy} onDone={() => setView("settings")} />
        )}
        {view === "welcome" && settings && (
          <AccessView
            settings={settings}
            firstRun
            setBusy={setBusy}
            onDone={async () => {
              const s = await api.saveSettings({ ...settings, onboarded: true });
              setSettings(s);
              setView("command");
            }}
          />
        )}
      </div>
    </div>
  );
}

/** Cycles example commands through the placeholder, typed out like a
 *  person would, while the box is empty. */
function useTypedPlaceholder(active: boolean): string {
  const base = "Type or dictate a command";
  const [shown, setShown] = useState(base);
  useEffect(() => {
    if (!active || reducedMotion()) {
      setShown(base);
      return;
    }
    let cancelled = false;
    let timer: number | undefined;
    const wait = (ms: number) => new Promise<void>((r) => (timer = window.setTimeout(r, ms)));
    (async () => {
      setShown(base);
      await wait(2600);
      for (let i = 0; !cancelled; i = (i + 1) % EXAMPLES.length) {
        const ex = EXAMPLES[i];
        for (let n = 1; n <= ex.length && !cancelled; n++) {
          setShown(ex.slice(0, n));
          await wait(38 + Math.random() * 40);
        }
        await wait(2200);
        for (let n = ex.length; n >= 0 && !cancelled; n--) {
          setShown(ex.slice(0, n));
          await wait(14);
        }
        await wait(350);
      }
    })();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [active]);
  return shown;
}

// ---- Command -----------------------------------------------------------------

function CommandView({
  open,
  onClose,
  onBusy,
  onSettings,
}: {
  open: boolean;
  onClose: () => void;
  onBusy: (busy: boolean) => void;
  onSettings: () => void;
}) {
  const [text, setText] = useState("");
  const [phase, setPhase] = useState<Phase>({ k: "ready" });
  const [asking, setAsking] = useState<Asking | null>(null);
  // The last command, echoed above its result once the box is cleared.
  const [said, setSaid] = useState<string | null>(null);
  const [heard, setHeard] = useState<string | null>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const ruleRef = useRef<HTMLSpanElement>(null);
  const planShownAt = useRef(0);

  const focusInput = useCallback(() => {
    // Focus after the window has become key, or macOS drops it.
    requestAnimationFrame(() => {
      const el = inputRef.current;
      if (!el) return;
      el.focus({ preventScroll: true });
      el.setSelectionRange(el.value.length, el.value.length);
    });
  }, []);

  useEffect(() => {
    const onFocus = () => {
      // A finished result doesn't carry over to the next time the window opens.
      setPhase((p) => {
        const live = p.k === "outcome" && (p.outcome.status === "plan" || p.outcome.status === "ambiguous");
        if (p.k === "ready" || p.k === "parsing" || p.k === "running" || live) return p;
        setSaid(null);
        return { k: "ready" };
      });
      focusInput();
    };
    window.addEventListener("voxi-focus", onFocus);
    window.addEventListener("focus", focusInput);
    focusInput();
    return () => {
      window.removeEventListener("voxi-focus", onFocus);
      window.removeEventListener("focus", focusInput);
    };
  }, [focusInput]);

  // Grow the text box with multi-line commands.
  useLayoutEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    const fit = () => {
      el.style.height = "0px";
      el.style.height = `${el.scrollHeight}px`;
    };
    fit();
    document.fonts.ready.then(fit);
  }, [text]);

  const submit = useCallback(
    async (command: string, picked: Record<string, string> = {}) => {
      if (!command.trim()) return;
      // Clear the box straight away so the next command starts fresh; the
      // command stays visible as the echo above its result.
      setSaid(command.trim().replace(/[.!?]+$/, ""));
      setText("");
      setPhase({ k: "parsing" });
      setHeard(null);
      try {
        const outcome = await api.plan(command, picked);
        setPhase({ k: "outcome", outcome });
        if (outcome.status === "plan") planShownAt.current = performance.now();
        if (outcome.status === "ambiguous") {
          // The answer will be dictated too.
          setAsking({ command, choices: picked, name: outcome.name, candidates: outcome.candidates });
        } else {
          setAsking(null);
        }
        // Focus stays in the box for every outcome, previews included, so
        // the next thing dictated always lands there. Enter runs a preview.
        focusInput();
      } catch (e) {
        setPhase({ k: "error", message: String(e) });
      }
    },
    [focusInput],
  );

  const pick = useCallback(
    (index: number) => {
      if (!asking) return;
      const path = asking.candidates[index];
      if (!path) return;
      submit(asking.command, { ...asking.choices, [asking.name]: path });
    },
    [asking, submit],
  );

  const stopAsking = useCallback(() => {
    setAsking(null);
  }, []);

  const confirm = useCallback(async () => {
    if (phase.k !== "outcome" || phase.outcome.status !== "plan") return;
    const plan = phase.outcome;
    setPhase({ k: "running", title: plan.title });
    try {
      const result = await api.confirm(plan.id);
      setPhase({ k: "done", result, wasUndo: plan.isUndo });
      focusInput();
    } catch (e) {
      setPhase({ k: "error", message: String(e) });
    }
  }, [phase, focusInput]);

  const reset = useCallback(() => {
    if (phase.k === "outcome" && phase.outcome.status === "plan") api.cancel();
    stopAsking();
    setHeard(null);
    setSaid(null);
    setPhase({ k: "ready" });
    focusInput();
  }, [phase, focusInput, stopAsking]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        if (phase.k === "outcome" || phase.k === "error" || phase.k === "done") reset();
        else onClose();
      } else if (e.key === "," && e.metaKey) {
        e.preventDefault();
        onSettings();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [phase, reset, onSettings, onClose]);

  const answer = (said: string) => {
    if (!asking) return;
    const choice = readChoice(said, asking.candidates);
    if (choice && "index" in choice) return pick(choice.index);
    if (choice && "cancel" in choice) return reset();
    // Not an answer: say so, and keep listening. A full new command (one
    // the parser would act on) needs Esc first, so a misheard number never
    // runs as a command.
    setHeard(said.trim().replace(/[.!?]+$/, ""));
    setText("");
    focusInput();
  };

  const onInputKey = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // Up in an empty box brings back the last command to edit.
    if (e.key === "ArrowUp" && text === "" && said && !asking) {
      e.preventDefault();
      setText(said);
      return;
    }
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      if (asking) return answer(text);
      const previewing = phase.k === "outcome" && phase.outcome.status === "plan";
      if (previewing && text.trim() === "") {
        // Enter on an empty box runs the preview. A held key, or an Enter
        // landing right as the preview appears, doesn't count: it was meant
        // for the command, not as a yes.
        if (e.repeat || performance.now() - planShownAt.current < 400) return;
        return confirm();
      }
      submit(text);
    }
  };

  // A keystroke presses the red rule; a dictated burst sweeps ink along it
  // and fades the new words in.
  const inkFeedback = (added: number) => {
    if (reducedMotion() || added <= 0) return;
    const rule = ruleRef.current;
    if (added > 3) {
      rule?.animate(
        [
          { backgroundPosition: "100% 0", transform: "scaleY(2)" },
          { backgroundPosition: "0% 0", transform: "scaleY(1)" },
        ],
        { duration: 520, easing: "cubic-bezier(.2,.8,.2,1)" },
      );
      inputRef.current?.animate([{ opacity: 0.25 }, { opacity: 1 }], { duration: 280, easing: "ease-out" });
    } else {
      rule?.animate([{ transform: "scaleY(1.9)" }, { transform: "scaleY(1)" }], { duration: 170, easing: "ease-out" });
    }
  };

  const onChange = (v: string) => {
    inkFeedback(v.length - text.length);
    setText(v);
    // While asking, the box holds the answer; the question stays up.
    if (asking) return;
    // A pending plan is dropped as soon as a new command starts, so Enter can
    // never run a preview the box no longer matches. Other results (a list,
    // a "Done") stay visible while the next command is dictated.
    if (phase.k === "outcome" && phase.outcome.status === "plan") {
      api.cancel();
      setSaid(null);
      setPhase({ k: "ready" });
    }
  };

  const fill = (example: string) => {
    setAsking(null);
    setText(example);
    setPhase({ k: "ready" });
    focusInput();
  };

  const undo = () => {
    setAsking(null);
    submit("undo last", {});
  };

  const busy = phase.k === "parsing" || phase.k === "running";
  useEffect(() => {
    onBusy(busy);
  }, [busy, onBusy]);
  const placeholder = useTypedPlaceholder(open && phase.k === "ready" && text === "");

  return (
    <>
      <div className="head" data-busy={busy || undefined}>
        <span className="rule" ref={ruleRef} aria-hidden="true" />
        <textarea
          ref={inputRef}
          className="command"
          rows={1}
          value={text}
          spellCheck={false}
          autoCorrect="off"
          autoCapitalize="off"
          placeholder={asking ? "Say or type a number" : placeholder}
          aria-label={asking ? "Which one?" : "Command"}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={onInputKey}
        />
      </div>
      {said && phase.k !== "ready" && (
        <p className="said" key={said}>
          “{said}”
        </p>
      )}
      <Body
        key={phaseKey(phase)}
        phase={phase}
        asking={asking}
        heard={heard}
        onConfirm={confirm}
        onCancel={reset}
        onPick={pick}
        onFill={fill}
        onUndo={undo}
      />
      <Footer phase={phase} asking={!!asking} onSettings={onSettings} />
    </>
  );
}

function phaseKey(p: Phase): string {
  if (p.k !== "outcome") return p.k;
  const o = p.outcome;
  return `outcome-${o.status}-${o.status === "plan" ? o.id : o.status === "ambiguous" ? o.name : ""}`;
}

/** Row index for the staggered "inked onto the card" entrance. */
const row = (i: number) => ({ "--i": Math.min(i, 12) }) as React.CSSProperties;

function Tick() {
  return (
    <svg className="tick" viewBox="0 0 20 20" aria-hidden="true">
      <path d="M4 10.5l4 4 8-9" />
    </svg>
  );
}

function Body(props: {
  phase: Phase;
  asking: Asking | null;
  heard: string | null;
  onConfirm: () => void;
  onCancel: () => void;
  onPick: (index: number) => void;
  onFill: (s: string) => void;
  onUndo: () => void;
}) {
  const { phase } = props;
  if (phase.k === "ready" || phase.k === "parsing") return null;

  if (phase.k === "running") {
    return (
      <section className="body">
        <p className="title">{phase.title}</p>
        <p className="quiet">Working…</p>
      </section>
    );
  }

  if (phase.k === "error") {
    return (
      <section className="body">
        <p className="title warn nudge">{phase.message}</p>
      </section>
    );
  }

  if (phase.k === "done") {
    const r = phase.result;
    return (
      <section className="body">
        <p className={r.error ? "title warn nudge" : "title"}>
          {!r.error && <Tick />}
          {r.summary}
        </p>
        {r.error && <p className="note">{r.error}</p>}
        <div className="actions">
          {r.undoable && (
            <button className="secondary" onClick={props.onUndo}>
              Undo
            </button>
          )}
        </div>
      </section>
    );
  }

  const o = phase.outcome;
  switch (o.status) {
    case "not_understood":
      return (
        <section className="body">
          <p className="title nudge">Didn't understand: “{o.text}”</p>
          <p className="quiet">Commands start with what to do. For example:</p>
          <ul className="examples">
            {EXAMPLES.map((ex, i) => (
              <li key={ex} style={row(i)}>
                <button className="example" onClick={() => props.onFill(ex)}>
                  {ex}
                </button>
              </li>
            ))}
          </ul>
        </section>
      );
    case "problem":
      return (
        <section className="body">
          <p className="title warn nudge">{o.message}</p>
        </section>
      );
    case "ambiguous":
      return (
        <section className="body">
          <p className="title">Which “{o.name}”?</p>
          <p className="quiet">Say a number, or click one.</p>
          <ol className="ruled choices">
            {o.candidates.map((c, i) => (
              <li key={c} style={row(i)}>
                <button className="row-button" onClick={() => props.onPick(i)} aria-label={`${i + 1}: ${c}`}>
                  <span className="num" aria-hidden="true">
                    {i + 1}
                  </span>
                  <span className="primary">{c.split("/").pop()}</span>
                  <span className="detail">{tildify(c.slice(0, c.lastIndexOf("/")))}</span>
                </button>
              </li>
            ))}
          </ol>
          {props.heard && (
            <p className="quiet nudge" key={props.heard}>
              Heard “{props.heard}”. Say a number from 1 to {o.candidates.length}, or “cancel”.
            </p>
          )}
        </section>
      );
    case "info":
      return (
        <section className="body">
          <p className="title">{o.title}</p>
          {o.summary && <p className="quiet">{o.summary}</p>}
          {o.rows.length > 0 && (
            <ul className="ruled scroll">
              {o.rows.map((r, i) => (
                <li key={i} style={row(i)}>
                  <span className="primary">{r.name}</span>
                  <span className="detail">{r.detail}</span>
                </li>
              ))}
            </ul>
          )}
          {o.total > o.rows.length && o.rows.length > 0 && <p className="quiet">Showing {o.rows.length} of {o.total}.</p>}
        </section>
      );
    case "plan": {
      const more = o.total - o.items.length;
      return (
        <section className="body">
          <p className="title">{o.title}</p>
          <ul className="ruled scroll">
            {o.items.map((it, i) => (
              <li key={i} data-kind={it.kind} style={row(i)}>
                <span className="primary">{it.primary}</span>
                <span className="detail">{it.detail}</span>
                {it.note && <span className="note">{it.note}</span>}
              </li>
            ))}
            {more > 0 && (
              <li className="more" style={row(o.items.length)}>
                and {more} more {more === 1 ? "step" : "steps"}
              </li>
            )}
          </ul>
          {o.warnings.map((w) => (
            <p key={w} className="quiet">
              {w}
            </p>
          ))}
          <div className="actions">
            <button className="primary-button" onClick={props.onConfirm}>
              {o.isUndo ? "Undo" : `Run ${o.total} ${o.total === 1 ? "change" : "changes"}`}
              <kbd>↩</kbd>
            </button>
            <button className="secondary" onClick={props.onCancel}>
              Cancel <kbd>esc</kbd>
            </button>
          </div>
        </section>
      );
    }
  }
}

function Footer({ phase, asking, onSettings }: { phase: Phase; asking: boolean; onSettings: () => void }) {
  const hint = asking
    ? "Enter to choose. Esc to cancel."
    : phase.k === "parsing"
      ? "Reading…"
      : phase.k === "ready"
        ? "Enter to preview. Shift-Enter for a new line."
        : phase.k === "outcome" && phase.outcome.status === "plan"
          ? "Enter to run. Nothing changes until then."
          : phase.k === "outcome" && phase.outcome.status === "ambiguous"
            ? "Esc to go back"
            : "Say the next command. Esc clears this.";
  return (
    <footer className="foot">
      <span>{hint}</span>
      <button className="link" onClick={onSettings} title="Settings (⌘,)">
        Settings <kbd>⌘,</kbd>
      </button>
    </footer>
  );
}

// ---- Shared frame ------------------------------------------------------------

/** Every screen uses the command window's layout: a title on the red rule,
 *  the content, and a quiet footer line. */
function Sheet(props: {
  title: string;
  aside?: React.ReactNode;
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
      <section className="body sheet-body">{props.children}</section>
      <footer className="foot">
        <span>{props.foot}</span>
        {props.footRight}
      </footer>
    </>
  );
}

// ---- Settings ----------------------------------------------------------------

const KEY_NAMES: Record<string, string> = { " ": "Space", ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right" };

function comboFromEvent(e: React.KeyboardEvent): string | null {
  if (["Meta", "Control", "Alt", "Shift"].includes(e.key)) return null;
  const mods = [e.ctrlKey && "Control", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(Boolean);
  if (mods.length === 0) return null;
  let key = e.code.startsWith("Key") ? e.code.slice(3) : e.code.startsWith("Digit") ? e.code.slice(5) : e.code;
  key = KEY_NAMES[e.key] ?? key;
  return [...mods, key].join("+");
}

type Tab = "general" | "shortcuts" | "folders";
const TABS: [Tab, string][] = [
  ["general", "General"],
  ["shortcuts", "Shortcuts"],
  ["folders", "Folders"],
];

/** Settings save as they change: no Save button. */
function SettingsView({
  settings,
  onSaved,
  onClose,
  onAccess,
  setBusy,
}: {
  settings: Settings;
  onSaved: (s: Settings) => void;
  onClose: () => void;
  onAccess: () => void;
  setBusy: (b: boolean) => void;
}) {
  const [tab, setTab] = useState<Tab>("general");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const persist = async (next: Settings): Promise<boolean> => {
    try {
      onSaved(await api.saveSettings(next));
      setError(null);
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    }
  };

  return (
    <Sheet
      title="Settings"
      aside={
        <div className="tabs" role="tablist">
          {TABS.map(([id, label]) => (
            <button key={id} role="tab" aria-selected={tab === id} onClick={() => setTab(id)}>
              {label}
            </button>
          ))}
        </div>
      }
      foot={error ? <span className="warn">{error}</span> : "Changes save as you go. Esc to go back."}
      footRight={
        <button className="link" onClick={onClose}>
          Done
        </button>
      }
    >
      <div key={tab} className="tab-panel">
        {tab === "general" && <GeneralTab settings={settings} persist={persist} onAccess={onAccess} />}
        {tab === "shortcuts" && <ShortcutsTab settings={settings} persist={persist} />}
        {tab === "folders" && <FoldersTab settings={settings} persist={persist} setBusy={setBusy} />}
      </div>
    </Sheet>
  );
}

type TabProps = { settings: Settings; persist: (s: Settings) => Promise<boolean> };

function GeneralTab({ settings, persist, onAccess }: TabProps & { onAccess: () => void }) {
  const [recording, setRecording] = useState(false);
  return (
    <ul className="prefs">
      <li className="pref">
        <span className="pref-label">Open Voxi</span>
        <button
          className={recording ? "hotkey recording" : "hotkey"}
          onClick={() => setRecording(true)}
          onBlur={() => setRecording(false)}
          title="Click, then press new keys. Avoid Fn, which Wispr Flow uses."
          onKeyDown={(e) => {
            if (!recording) return;
            e.preventDefault();
            e.stopPropagation(); // Esc here cancels recording, not Settings.
            if (e.key === "Escape") return setRecording(false);
            const combo = comboFromEvent(e);
            if (combo) {
              setRecording(false);
              persist({ ...settings, hotkey: combo });
            }
          }}
        >
          {recording ? "Press new keys" : prettyHotkey(settings.hotkey)}
        </button>
      </li>
      <li className="pref">
        <label className="pref-label" htmlFor="login">
          Open at login
        </label>
        <input
          id="login"
          className="switch"
          type="checkbox"
          checked={settings.startAtLogin}
          onChange={(e) => persist({ ...settings, startAtLogin: e.target.checked })}
        />
      </li>
      <li className="pref">
        <span className="pref-label">Folder access</span>
        <button className="secondary small" onClick={onAccess}>
          Review
        </button>
      </li>
      <UpdatesRow />
    </ul>
  );
}

const mb = (bytes: number) => (bytes / 1_000_000).toFixed(1);

/** The installed version, and a button that fetches and installs the latest
 *  release, with a progress bar and its notes while it downloads. */
function UpdatesRow() {
  const [version, setVersion] = useState("");
  const [state, setState] = useState<UpdateState>({ k: "idle" });

  useEffect(() => {
    appVersion().then(setVersion);
  }, []);

  const busy = state.k === "checking" || state.k === "downloading" || state.k === "restarting";
  const status =
    state.k === "checking"
      ? "Checking…"
      : state.k === "latest"
        ? "You're up to date."
        : state.k === "downloading"
          ? state.total
            ? `Downloading ${state.version}: ${mb(state.done)} of ${mb(state.total)} MB`
            : `Downloading ${state.version}…`
          : state.k === "restarting"
            ? `Installed ${state.version}. Restarting…`
            : state.k === "error"
              ? state.message
              : null;
  const fraction = state.k === "downloading" && state.total ? Math.min(state.done / state.total, 1) : null;
  const notes = state.k === "downloading" || state.k === "restarting" ? state.notes : [];

  return (
    <li className="pref updates">
      <span className="pref-label">Version {version}</span>
      <button className="secondary small" disabled={busy} onClick={() => checkAndInstall(setState)}>
        Check for updates
      </button>
      {status && (
        <div className="update-status" role="status">
          <span className={state.k === "error" ? "hint warn" : "hint"}>{status}</span>
          {(state.k === "downloading" || state.k === "restarting") && (
            <div
              className="progress"
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={fraction === null ? undefined : Math.round(fraction * 100)}
              data-indeterminate={fraction === null && state.k === "downloading" ? "" : undefined}
            >
              <span style={{ width: `${state.k === "restarting" ? 100 : (fraction ?? 0.3) * 100}%` }} />
            </div>
          )}
          {notes.length > 0 && (
            <ul className="notes">
              {notes.map((n) => (
                <li key={n}>{n}</li>
              ))}
            </ul>
          )}
        </div>
      )}
    </li>
  );
}

/** Saved phrases that run several commands at once, with one preview and one
 *  undo: "run my usuals" -> move today's screenshots, clear installers. Each
 *  shortcut stays folded to its phrase until opened; Done saves it. */
function ShortcutsTab({ settings, persist }: TabProps) {
  const [list, setList] = useState<Shortcut[]>(settings.shortcuts);
  const [editing, setEditing] = useState<number | null>(null);
  const update = (i: number, s: Shortcut) => setList((l) => l.map((x, j) => (j === i ? s : x)));
  const count = (s: Shortcut) => s.commands.filter((c) => c.trim()).length;

  const commit = async (next: Shortcut[]) => {
    if (await persist({ ...settings, shortcuts: next })) setEditing(null);
  };

  return (
    <div className="shortcuts">
      <p className="hint">Say a shortcut to run all its commands, with one preview and one undo.</p>
      <ul className="prefs">
        {list.map((s, i) =>
          editing === i ? (
            <li className="shortcut" key={i}>
              <div className="shortcut-head">
                <input
                  className="phrase"
                  value={s.phrase}
                  placeholder="run my usuals"
                  aria-label="Phrase to say"
                  autoFocus={!s.phrase}
                  onChange={(e) => update(i, { ...s, phrase: e.target.value })}
                />
                <button className="link strong" onClick={() => commit(list)}>
                  Done
                </button>
              </div>
              <textarea
                className="plain"
                rows={Math.max(2, s.commands.length)}
                placeholder={"move screenshots from today to Screenshots\nclean up Downloads older than 90 days"}
                aria-label="Commands, one per line"
                value={s.commands.join("\n")}
                onChange={(e) => update(i, { ...s, commands: e.target.value.split("\n") })}
              />
              <button
                className="link remove"
                onClick={() => {
                  const next = list.filter((_, j) => j !== i);
                  setList(next);
                  commit(next);
                }}
              >
                Remove shortcut
              </button>
            </li>
          ) : (
            <li className="pref" key={i}>
              <button className="shortcut-row" onClick={() => setEditing(i)}>
                <span className="phrase-text">{s.phrase || "Untitled"}</span>
                <span className="pref-summary">
                  {count(s)} {count(s) === 1 ? "command" : "commands"}
                </span>
              </button>
            </li>
          ),
        )}
      </ul>
      {editing === null && (
        <button
          className="link strong add"
          onClick={() => {
            setList((l) => [...l, { phrase: "", commands: [] }]);
            setEditing(list.length);
          }}
        >
          + Add shortcut
        </button>
      )}
    </div>
  );
}

/** The four standard folders are always allowed; more can be added. */
/** "~/Work" from "/Users/me/Work/", the form allowed_folders returns. */
const shownFolder = (path: string) => path.replace(/^\/Users\/[^/]+/, "~").replace(/\/$/, "");

function FoldersTab({ settings, persist, setBusy }: TabProps & { setBusy: (b: boolean) => void }) {
  const [folders, setFolders] = useState<string[]>([]);
  const [adding, setAdding] = useState("");
  const [note, setNote] = useState<string | null>(null);
  const extra = settings.extraFolders.filter((f) => f.trim());

  useEffect(() => {
    api.allowedFolders().then(setFolders);
  }, [settings]);

  const add = async (path: string) => {
    if (!path) return;
    const shown = shownFolder(path);
    if (folders.includes(shown) || extra.some((x) => shownFolder(x) === shown)) {
      setAdding("");
      setNote(`“${shown}” is already allowed.`);
      return;
    }
    if (await persist({ ...settings, extraFolders: [...extra, path] })) {
      const now = await api.allowedFolders();
      setFolders(now);
      setAdding("");
      setNote(now.includes(shown) ? null : `Couldn't find “${path}”. Check the path and try again.`);
    }
  };

  // The picker takes focus, which would otherwise close the window.
  const choose = async () => {
    setBusy(true);
    try {
      const path = await api.pickFolder();
      if (path) await add(path);
    } catch (e) {
      setNote(`Couldn't open the folder picker: ${e}`);
    } finally {
      setBusy(false);
    }
  };

  const builtIn = folders.filter((f) => !extra.some((x) => shownFolder(x) === f));

  return (
    <div>
      <p className="hint">Voxi only changes files inside these folders.</p>
      <ul className="prefs">
        {builtIn.map((f) => (
          <li className="pref" key={f}>
            <span>{f}</span>
            <span className="pref-summary">Always</span>
          </li>
        ))}
        {extra.map((f) => (
          <li className="pref" key={f}>
            <span>{f}</span>
            <button className="link remove" onClick={() => persist({ ...settings, extraFolders: extra.filter((x) => x !== f) })}>
              Remove
            </button>
          </li>
        ))}
        <li className="pref">
          <input
            className="path"
            value={adding}
            placeholder="Add a folder, like /Volumes/External"
            aria-label="Folder path to add"
            onChange={(e) => setAdding(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                add(adding.trim());
              }
            }}
          />
          <button className="secondary small" disabled={!adding.trim()} onClick={() => add(adding.trim())}>
            Add
          </button>
          <button className="secondary small" onClick={choose}>
            Choose…
          </button>
        </li>
      </ul>
      {note && <p className="hint warn">{note}</p>}
    </div>
  );
}

// ---- Folder access ---------------------------------------------------------------

/** Asks macOS for every folder up front, so commands never stop to ask. On
 *  first launch this is the welcome page. */
function AccessView({
  settings,
  firstRun,
  onDone,
  setBusy,
}: {
  settings: Settings;
  firstRun: boolean;
  onDone: () => void;
  setBusy: (b: boolean) => void;
}) {
  const [folders, setFolders] = useState<FolderAccess[] | null>(null);
  const [asking, setAsking] = useState(false);

  useEffect(() => {
    if (firstRun) return;
    // From Settings: check right away. Granted folders don't prompt again.
    request();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !firstRun) {
        e.preventDefault();
        onDone();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [firstRun, onDone]);

  async function request() {
    setAsking(true);
    // macOS's prompts take focus; don't let that close the window.
    setBusy(true);
    try {
      setFolders(await api.requestFolderAccess());
    } finally {
      setBusy(false);
      setAsking(false);
    }
  }

  const missing = folders?.filter((f) => !f.allowed) ?? [];
  const allSet = folders !== null && missing.length === 0;

  return (
    <Sheet
      title={firstRun ? "Welcome to Voxi" : "Folder access"}
      foot={firstRun ? `Press ${prettyHotkey(settings.hotkey)} anywhere to open Voxi.` : "Esc to go back."}
      footRight={
        <button className="link" onClick={onDone}>
          {firstRun && !allSet ? "Skip for now" : "Done"}
        </button>
      }
    >
      <p className="access-lead">
        {firstRun
          ? "Voxi moves, renames, and tidies files when you tell it to. macOS asks once for each folder; allow them now and commands won't stop to ask."
          : "macOS asks once for each folder. Voxi never touches anything outside them."}
      </p>
      <ul className="prefs">
        {(folders ?? DEFAULT_FOLDERS).map((f, i) => (
          <li className="pref access-row" key={f.path} style={row(i)}>
            <span>
              <span className="pref-label">{f.name}</span> <span className="pref-path">{f.path}</span>
            </span>
            <span className={folders === null ? "pref-summary" : f.allowed ? "pref-summary ok" : "pref-summary warn"}>
              {folders === null ? "Not asked yet" : f.allowed ? "Allowed" : "Not allowed"}
            </span>
          </li>
        ))}
      </ul>
      {missing.length > 0 && (
        <p className="hint warn">
          Turn on Voxi for {missing.map((f) => f.name).join(", ")} in System Settings, under Files and Folders, then check again.
        </p>
      )}
      <div className="actions">
        {allSet ? (
          <button className="primary-button" autoFocus onClick={onDone}>
            {firstRun ? "Start using Voxi" : "Done"}
          </button>
        ) : (
          <button className="primary-button" autoFocus disabled={asking} onClick={request}>
            {asking ? "Waiting for macOS…" : folders === null ? "Allow access" : "Check again"}
          </button>
        )}
        {missing.length > 0 && (
          <button className="secondary" onClick={() => api.openPrivacySettings()}>
            Open System Settings
          </button>
        )}
      </div>
    </Sheet>
  );
}

const DEFAULT_FOLDERS: FolderAccess[] = ["Desktop", "Downloads", "Documents", "Pictures"].map((name) => ({
  name,
  path: `~/${name}`,
  allowed: false,
}));
