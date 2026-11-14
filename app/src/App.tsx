import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { api, inTauri, tildify, type ScheduledTask, type FolderAccess, type Outcome, type Reports, type ResultScope, type RunResult, type Settings, type Shortcut } from "./api";
import { readChoice } from "./choice";
import examples from "./examples.json";
import { appVersion, checkAndInstall, type UpdateState } from "./updates";

type View = "command" | "settings" | "welcome";
type Phase =
  | { k: "ready" }
  | { k: "parsing" }
  | { k: "outcome"; outcome: Outcome }
  | { k: "running"; title: string }
  | { k: "done"; result: RunResult; wasUndo: boolean }
  | { k: "error"; message: string };

/** Waiting for the answer to "which one?": the command being planned and
 *  the picks made so far (one command can need several). */
type Asking = { command: string; choices: Record<string, string>; name: string; candidates: string[]; folders: boolean };

/** Something Voxi can learn from what just happened, offered once. */
type Offer =
  | { k: "folder"; name: string; path: string }
  | { k: "phrase"; phrase: string; command: string };

/** How long a missed phrase stays teachable by the command that follows it. */
const TEACH_WINDOW_MS = 3 * 60_000;

/** Example commands, one per kind of feature. A Rust test checks each parses. */
const EXAMPLES: readonly string[] = examples;

/** A few examples to click, picked once when shown. */
function ExampleList({ onFill }: { onFill: (text: string) => void }) {
  const [picked] = useState(() => someExamples(4));
  return (
    <ul className="examples">
      {picked.map((ex, i) => (
        <li key={ex} style={row(i)}>
          <button className="example" onClick={() => onFill(ex)}>
            {ex}
          </button>
        </li>
      ))}
    </ul>
  );
}

/** `count` examples starting at a random one, so each opening shows different ones. */
function someExamples(count: number): string[] {
  const start = Math.floor(Math.random() * EXAMPLES.length);
  return Array.from({ length: Math.min(count, EXAMPLES.length) }, (_, i) => EXAMPLES[(start + i) % EXAMPLES.length]);
}

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
const CLOSE_MS = 140;

export default function App() {
  const [view, setView] = useState<View>("command");
  const [settings, setSettings] = useState<Settings | null>(null);
  const [open, setOpen] = useState(!inTauri);
  const [pendingShow, setPendingShow] = useState<number | null>(null);
  const [commandSession, setCommandSession] = useState(0);
  const [fontsReady, setFontsReady] = useState(false);
  const [height, setHeight] = useState<number | null>(null);
  const [glide, setGlide] = useState(false);
  const innerRef = useRef<HTMLDivElement>(null);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const showId = useRef(0);
  const awaitingShow = useRef<number | null>(null);

  const cancelClose = useCallback(() => {
    if (closeTimer.current !== null) clearTimeout(closeTimer.current);
    closeTimer.current = null;
  }, []);
  useEffect(() => cancelClose, [cancelClose]);

  // Plays the close animation, then hides the window. The card stays in its
  // closed state while hidden, so the next show animates in from there.
  const close = useCallback(() => {
    cancelClose();
    const requestId = showId.current;
    awaitingShow.current = null;
    setOpen(false);
    setPendingShow(null);
    if (inTauri) void invoke("dismiss_window", { requestId, animate: !reducedMotion() });
    closeTimer.current = setTimeout(() => {
      closeTimer.current = null;
      void api.hide(requestId);
    }, reducedMotion() ? 0 : CLOSE_MS);
  }, [cancelClose]);

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
    let disposed = false;
    const unShown = listen<{ requestId: number; mode: View }>("voxi://shown", (e) => {
      const { mode, requestId } = e.payload;
      cancelClose();
      showId.current = requestId;
      awaitingShow.current = requestId;
      setCommandSession(requestId);
      setView((v) => (mode === "command" && v === "welcome" ? v : mode));
      shownAt.current = performance.now();
      setOpen(false);
      setPendingShow(requestId);
    });
    const unPresented = listen<number>("voxi://presented", ({ payload: requestId }) => {
      if (requestId !== showId.current || awaitingShow.current !== requestId) return;
      awaitingShow.current = null;
      setPendingShow(null);
      setOpen(true);
      shownAt.current = performance.now();
      window.dispatchEvent(new Event("voxi-focus"));
    });
    const unHide = listen("voxi://hide", close);
    Promise.all([unShown, unPresented, unHide]).then(() => { if (!disposed) void invoke("frontend_ready"); });
    return () => {
      disposed = true;
      unShown.then((f) => f());
      unPresented.then((f) => f());
      unHide.then((f) => f());
    };
  }, [close, cancelClose]);

  useEffect(() => {
    let disposed = false;
    document.fonts.ready.then(() => { if (!disposed) setFontsReady(true); });
    return () => { disposed = true; };
  }, []);

  // Layout is measured after settings and fonts settle, while the native window
  // is still hidden. The acknowledgement resizes and reveals it in one operation.
  useLayoutEffect(() => {
    if (!inTauri || pendingShow === null || !settings || !fontsReady || !innerRef.current) return;
    const requestId = pendingShow;
    const height = Math.ceil(innerRef.current.offsetHeight);
    void invoke("present_window", { requestId, height, animate: !reducedMotion() });
  }, [pendingShow, settings, fontsReady, view]);

  // The window is the frosted card, so it is sized to the content and macOS
  // animates the resize. In a browser preview the card glides instead.
  useLayoutEffect(() => {
    const el = innerRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => {
      const h = Math.ceil(el.offsetHeight);
      if (inTauri) invoke("fit_window", { height: h, animate: !reducedMotion() });
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
            key={commandSession}
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
            setBusy={setBusy}
          />
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
      for (let i = Math.floor(Math.random() * EXAMPLES.length); !cancelled; i = (i + 1) % EXAMPLES.length) {
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
  const [resultScope, setResultScope] = useState<ResultScope | null>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const ruleRef = useRef<HTMLSpanElement>(null);
  const planShownAt = useRef(0);
  const requestId = useRef(0);
  const [searched, setSearched] = useState(0);
  // The command behind the result on screen, exactly as entered.
  const lastCommand = useRef("");
  // The last phrase Voxi didn't understand, which the next run can teach.
  const missed = useRef<{ text: string; at: number } | null>(null);
  const [offer, setOffer] = useState<Offer | null>(null);
  // A folder picked for an ambiguous name, offered as a nickname after the plan.
  const pickedFolder = useRef<{ name: string; path: string } | null>(null);

  useEffect(() => () => {
    ++requestId.current;
    api.cancel();
    void api.clearResultScope();
  }, []);

  useEffect(() => {
    if (phase.k !== "parsing") return;
    const request = requestId.current;
    const timer = window.setInterval(() => {
      api.searchProgress(request).then((count) => {
        if (request === requestId.current) setSearched(count);
      }).catch(() => {});
    }, 150);
    return () => window.clearInterval(timer);
  }, [phase]);

  const focusInput = useCallback(() => {
    // Focus after the window has become key, or macOS drops it.
    requestAnimationFrame(() => {
      const el = inputRef.current;
      if (!el) return;
      el.focus({ preventScroll: true });
      el.setSelectionRange(el.value.length, el.value.length);
    });
  }, []);

  // Each native show request mounts a fresh command session before layout is
  // measured. Focus events only focus the input; they do not reset a live session.
  useEffect(() => {
    window.addEventListener("voxi-focus", focusInput);
    window.addEventListener("focus", focusInput);
    focusInput();
    return () => {
      window.removeEventListener("voxi-focus", focusInput);
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
      lastCommand.current = command.trim();
      setOffer(null);
      const folderPick = pickedFolder.current;
      pickedFolder.current = null;
      const request = ++requestId.current;
      setSearched(0);
      setPhase({ k: "parsing" });
      setHeard(null);
      try {
        const { outcome, scope } = await api.plan(command, picked, request);
        if (request !== requestId.current) return;
        setResultScope(scope);
        setPhase({ k: "outcome", outcome });
        if (outcome.status === "plan") planShownAt.current = performance.now();
        if (outcome.status === "not_understood") missed.current = { text: command.trim(), at: Date.now() };
        // Once the picked folder led somewhere, offer to remember it by name.
        if (folderPick && (outcome.status === "plan" || outcome.status === "info")) setOffer({ k: "folder", ...folderPick });
        if (outcome.status === "ambiguous") {
          // The answer will be dictated too.
          setAsking({ command, choices: picked, name: outcome.name, candidates: outcome.candidates, folders: outcome.folders });
        } else {
          setAsking(null);
        }
        // Focus stays in the box for every outcome, previews included, so
        // the next thing dictated always lands there. Enter runs a preview.
        focusInput();
      } catch (e) {
        if (request === requestId.current) setPhase({ k: "error", message: String(e) });
      }
    },
    [focusInput],
  );

  const pick = useCallback(
    (index: number) => {
      if (!asking) return;
      const path = asking.candidates[index];
      if (!path) return;
      if (asking.folders) pickedFolder.current = { name: asking.name, path };
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
      setResultScope(await api.resultScope().catch(() => null));
      setPhase({ k: "done", result, wasUndo: plan.isUndo });
      // A phrase Voxi missed, followed soon by a command that worked, can
      // be taught: next time the phrase runs this command.
      const miss = missed.current;
      missed.current = null;
      if (!result.error && !plan.isUndo && miss && Date.now() - miss.at < TEACH_WINDOW_MS && miss.text !== lastCommand.current) {
        setOffer({ k: "phrase", phrase: miss.text, command: lastCommand.current });
      }
      focusInput();
    } catch (e) {
      setPhase({ k: "error", message: String(e) });
    }
  }, [phase, focusInput]);

  const reset = useCallback(() => {
    const request = ++requestId.current;
    void Promise.all([api.cancel(), api.clearResultScope()]).catch((e) => {
      if (request === requestId.current) setPhase({ k: "error", message: String(e) });
    });
    stopAsking();
    setText("");
    setResultScope(null);
    setSearched(0);
    setHeard(null);
    setSaid(null);
    setOffer(null);
    lastCommand.current = "";
    missed.current = null;
    pickedFolder.current = null;
    setPhase({ k: "ready" });
    focusInput();
  }, [focusInput, stopAsking]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.isComposing) return;
      if (e.key === "Escape") {
        e.preventDefault();
        if (e.repeat || phase.k === "running") return;
        if (phase.k !== "ready" || text !== "" || resultScope) reset();
        else onClose();
      } else if (e.key === "," && e.metaKey) {
        e.preventDefault();
        onSettings();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [phase, text, resultScope, reset, onSettings, onClose]);

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
    return () => onBusy(false);
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
          writingsuggestions="false"
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
      {resultScope && phase.k !== "parsing" && <div className="result-scope" role="status">
        <span>{resultScope.count} results from {resultScope.label}{resultScope.visible < resultScope.count ? ` · ${resultScope.visible} displayed` : ""}</span>
        <button className="link" disabled={phase.k === "running"} onClick={() => {
          reset();
          void api.clearResultScope().then(() => setResultScope(null)).catch((e) => setPhase({ k: "error", message: String(e) }));
        }}>Clear scope</button>
      </div>}
      <Body
        searched={searched}
        key={`body-${phaseKey(phase)}`}
        phase={phase}
        asking={asking}
        heard={heard}
        onConfirm={confirm}
        onCancel={reset}
        onPick={pick}
        onFill={fill}
        onUndo={undo}
      />
      {!busy && offer && <OfferRow key={offer.k === "folder" ? offer.path : offer.phrase} offer={offer} onDone={() => { setOffer(null); focusInput(); }} />}
      {reportable(phase) && <ReportRow key={`report-${phaseKey(phase)}`} command={lastCommand.current} phase={phase} onDone={focusInput} />}
      <Footer phase={phase} asking={!!asking} onSettings={onSettings} />
    </>
  );
}

/** Results that can be reported as wrong: anything Voxi decided, not a pick list. */
function reportable(p: Phase): boolean {
  return p.k === "done" || (p.k === "outcome" && p.outcome.status !== "ambiguous");
}

/** What the person saw, small enough to keep in a report. */
function reportOutcome(p: Phase): unknown {
  if (p.k === "done") return { status: "done", summary: p.result.summary, error: p.result.error };
  if (p.k !== "outcome") return { status: p.k };
  const o = p.outcome;
  switch (o.status) {
    case "plan": return { status: o.status, title: o.title, total: o.total, items: o.items.slice(0, 5).map((i) => `${i.kind}: ${i.primary} ${i.detail}`) };
    case "info": return { status: o.status, title: o.title, summary: o.summary, total: o.total, rows: o.rows.slice(0, 5).map((r) => r.name) };
    case "problem": return { status: o.status, message: o.message };
    default: return { status: o.status };
  }
}

/** "Wrong result?": saves the command and what Voxi did with it on this Mac,
 *  with an optional note on what it should have done. Nothing is sent. */
function ReportRow({ command, phase, onDone }: { command: string; phase: Phase; onDone: () => void }) {
  const [state, setState] = useState<{ k: "idle" } | { k: "writing"; note: string } | { k: "saved"; count: number } | { k: "error"; message: string }>({ k: "idle" });
  if (!command) return null;
  const save = async (note: string) => {
    try {
      const r = await api.reportWrong(command, reportOutcome(phase), note);
      setState({ k: "saved", count: r.count });
    } catch (e) {
      setState({ k: "error", message: String(e) });
    }
    onDone();
  };
  if (state.k === "idle") return (
    <div className="learn">
      <button className="link" onClick={() => setState({ k: "writing", note: "" })}>Wrong result?</button>
    </div>
  );
  if (state.k === "saved") return <div className="learn" role="status"><span>Saved on this Mac. Nothing is sent. {state.count} {state.count === 1 ? "report" : "reports"} so far.</span></div>;
  if (state.k === "error") return <div className="learn" role="alert"><span className="warn">{state.message}</span></div>;
  return (
    <div className="learn writing">
      <input
        className="path"
        autoFocus
        value={state.note}
        placeholder="What should Voxi have done? (optional)"
        aria-label="What should Voxi have done?"
        onChange={(e) => setState({ k: "writing", note: e.target.value })}
        onKeyDown={(e) => {
          if (e.key === "Enter") { e.preventDefault(); e.stopPropagation(); save(state.note); }
        }}
      />
      <button className="secondary small" onClick={() => save(state.note)}>Save report</button>
      <button className="link" onClick={() => { setState({ k: "idle" }); onDone(); }}>Cancel</button>
    </div>
  );
}

/** One offer to learn from what just happened: a folder's nickname, or a
 *  phrase that should run the command that worked. */
function OfferRow({ offer, onDone }: { offer: Offer; onDone: () => void }) {
  const [state, setState] = useState<{ k: "asking" } | { k: "saved"; text: string } | { k: "error"; message: string }>({ k: "asking" });
  const accept = async () => {
    try {
      if (offer.k === "folder") {
        await api.rememberFolder(offer.name, offer.path);
        setState({ k: "saved", text: `Saved. “${offer.name}” now means ${tildify(offer.path)}.` });
      } else {
        await api.teachPhrase(offer.phrase, offer.command);
        setState({ k: "saved", text: `Saved. “${offer.phrase}” now runs “${offer.command}”. Change it under Shortcuts.` });
      }
    } catch (e) {
      setState({ k: "error", message: String(e) });
    }
  };
  if (state.k === "saved") return <div className="learn offer" role="status"><span>{state.text}</span></div>;
  if (state.k === "error") return <div className="learn offer" role="alert"><span className="warn">{state.message}</span></div>;
  const question = offer.k === "folder"
    ? <>Always use <strong>{tildify(offer.path)}</strong> for “{offer.name}”?</>
    : <>Next time you say “{offer.phrase}”, run this?</>;
  return (
    <div className="learn offer">
      <span>{question}</span>
      <span className="learn-buttons">
        <button className="secondary small" onClick={accept}>{offer.k === "folder" ? "Remember" : "Teach Voxi"}</button>
        <button className="link" onClick={onDone}>No thanks</button>
      </span>
    </div>
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
  searched: number;
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
  if (phase.k === "ready") return null;
  if (phase.k === "parsing") return (
    <section className="body" aria-live="polite">
      <p className="quiet">Searching… {props.searched.toLocaleString()} entries checked.</p>
      <div className="actions"><button className="secondary" onClick={props.onCancel}>Cancel</button></div>
    </section>
  );

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
          <p className="quiet">Use a filename, folder, or a specific group of files. For example:</p>
          <ExampleList onFill={props.onFill} />
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
          <p className="title">Choose a match for “{o.name}”</p>
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
            <ol className={`ruled scroll${o.numbered ? " numbered" : ""}`}>
              {o.rows.map((r, i) => (
                <li key={i} style={row(i)}>
                  {o.numbered && <span className="num" aria-hidden="true">{i + 1}</span>}
                  <span className="primary">{r.name}</span>
                  <span className="detail">{r.detail}</span>
                </li>
              ))}
            </ol>
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
              {o.repeats ? "Schedule repeating task" : o.scheduledFor ? "Schedule task" : o.isUndo ? "Undo" : `Run ${o.total} ${o.total === 1 ? "change" : "changes"}`}
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
          ? (phase.outcome.scheduledFor ? (phase.outcome.repeats ? "Enter to schedule. Later runs won't ask first. Esc to cancel." : "Enter to schedule. Esc to cancel.") : "Enter to run. Nothing changes until then.")
          : phase.k === "outcome" && phase.outcome.status === "ambiguous"
            ? "Esc to go back"
            : "Say the next command. Esc to go back.";
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

/** Everything a Settings tab can use. */
type TabContext = {
  settings: Settings;
  persist: (s: Settings) => Promise<boolean>;
  setBusy: (b: boolean) => void;
  activeTasks: number | null;
};

/** 16px line icons, drawn in the current text color. */
const ICONS = {
  general: <><circle cx="8" cy="8" r="2.2" /><path d="M8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M3.4 12.6l1.4-1.4M11.2 4.8l1.4-1.4" /></>,
  scheduled: <><circle cx="8" cy="8" r="6" /><path d="M8 4.5V8l2.5 1.5" /></>,
  shortcuts: <path d="M9 1.5 3.5 9H8l-1 5.5L12.5 7H8z" />,
  folders: <path d="M1.75 4.25c0-.7.55-1.25 1.25-1.25h3l1.5 1.5H13c.7 0 1.25.55 1.25 1.25v6c0 .7-.55 1.25-1.25 1.25H3c-.7 0-1.25-.55-1.25-1.25z" />,
  keyboard: <><rect x="1.5" y="4" width="13" height="8" rx="1.5" /><path d="M4 7h.01M6.5 7h.01M9 7h.01M11.5 7h.01M5 9.5h6" /></>,
  power: <><path d="M8 1.75V7.5" /><path d="M4.6 3.6a5.25 5.25 0 1 0 6.8 0" /></>,
  mic: <><rect x="5.75" y="1.5" width="4.5" height="8" rx="2.25" /><path d="M3.5 7.5a4.5 4.5 0 0 0 9 0M8 12v2.5" /></>,
  update: <><path d="M13.5 8A5.5 5.5 0 1 1 11.9 4.1" /><path d="M13.5 2v3h-3" /></>,
  shield: <path d="M8 1.75 2.75 3.75V7.5c0 3.1 2.2 5.6 5.25 6.75 3.05-1.15 5.25-3.65 5.25-6.75V3.75z" />,
  tag: <><path d="M1.75 8.4V2.75c0-.55.45-1 1-1H8.4l5.85 5.85-6.65 6.65z" /><circle cx="5" cy="5" r="1" /></>,
} as const;

function Icon({ name }: { name: keyof typeof ICONS }) {
  return <svg className="icon" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{ICONS[name]}</svg>;
}

/** Settings sections, in sidebar order. Add a tab by adding an entry here. */
const TABS = [
  {
    id: "general", label: "General", icon: "general", blurb: "How Voxi opens, updates, and keeps your data.",
    badge: () => null,
    render: (c: TabContext) => <GeneralTab settings={c.settings} persist={c.persist} />,
  },
  {
    id: "scheduled", label: "Scheduled", icon: "scheduled", blurb: "Commands set to run later or on repeat.",
    badge: (c: TabContext) => c.activeTasks || null,
    render: () => <ScheduledTab />,
  },
  {
    id: "shortcuts", label: "Shortcuts", icon: "shortcuts", blurb: "One phrase that runs several commands.",
    badge: (c: TabContext) => c.settings.shortcuts.length || null,
    render: (c: TabContext) => <ShortcutsTab settings={c.settings} persist={c.persist} />,
  },
  {
    id: "folders", label: "Folders", icon: "folders", blurb: "Where Voxi may change files, and your names for them.",
    badge: () => null,
    render: (c: TabContext) => <FoldersTab settings={c.settings} persist={c.persist} setBusy={c.setBusy} />,
  },
] as const satisfies readonly {
  id: string; label: string; icon: keyof typeof ICONS; blurb: string;
  badge: (c: TabContext) => number | null; render: (c: TabContext) => React.ReactNode;
}[];
type Tab = (typeof TABS)[number]["id"];

/** A titled group of rows on a soft card, like System Settings. */
function Group({ title, action, foot, children }: { title: string; action?: React.ReactNode; foot?: React.ReactNode; children: React.ReactNode }) {
  return <section className="group">
    <div className="group-head">
      <h3 className="group-title">{title}</h3>
      {action}
    </div>
    <ul className="group-rows">{children}</ul>
    {foot && <p className="group-foot">{foot}</p>}
  </section>;
}

/** One setting: an optional icon, a label with a short description, and its control. */
function Row({ icon, label, htmlFor, description, children, className }: {
  icon?: keyof typeof ICONS; label?: React.ReactNode; htmlFor?: string; description?: React.ReactNode;
  children?: React.ReactNode; className?: string;
}) {
  return <li className={className ? `row ${className}` : "row"}>
    {icon && <span className="row-icon"><Icon name={icon} /></span>}
    {label !== undefined && <div className="row-copy">
      {htmlFor ? <label className="row-label" htmlFor={htmlFor}>{label}</label> : <span className="row-label">{label}</span>}
      {description && <span className="row-desc">{description}</span>}
    </div>}
    {children}
  </li>;
}

const ACTIVE_STATUSES: ScheduledTask["status"][] = ["pending", "running", "paused"];

function ScheduledTab() {
  const [tasks, setTasks] = useState<ScheduledTask[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [working, setWorking] = useState<number | null>(null);
  const [showing, setShowing] = useState<"active" | "done">("active");
  useEffect(() => {
    let alive = true;
    const refresh = () => Promise.all([api.scheduledTasks(), api.schedulerNotice()]).then(([next, message]) => {
      if (alive) { setTasks(next); setNotice(message); setLoading(false); setError(null); }
    }).catch((e) => { if (alive) { setError(String(e)); setLoading(false); } });
    void refresh();
    const timer = window.setInterval(refresh, 2000);
    return () => { alive = false; clearInterval(timer); };
  }, []);
  const act = async (id: number, action: (id: number) => Promise<void>) => {
    setWorking(id);
    try { await action(id); setTasks(await api.scheduledTasks()); setError(null); }
    catch (e) { setError(String(e)); }
    finally { setWorking(null); }
  };
  const when = (task: ScheduledTask) => {
    const at = new Date(task.due).toLocaleString(undefined, { weekday: "short", month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
    if (!task.repeat) return at;
    const every = `${task.repeat[0].toUpperCase()}${task.repeat.slice(1)}`;
    return task.status === "pending" ? `${every} · next ${at}` : every;
  };
  const STATUS: Record<ScheduledTask["status"], [string, string]> = {
    pending: ["Scheduled", "ok"], running: ["Running", "ok"], paused: ["Paused", "muted"],
    completed: ["Done", "muted"], cancelled: ["Stopped", "muted"], failed: ["Failed", "warn"],
  };
  const active = tasks.filter((t) => ACTIVE_STATUSES.includes(t.status)).reverse();
  const done = tasks.filter((t) => !ACTIVE_STATUSES.includes(t.status)).reverse();
  const shown = showing === "active" ? active : done;
  const button = (task: ScheduledTask, label: string, busy: string, action: (id: number) => Promise<void>) =>
    <button className="link" disabled={working !== null} onClick={() => void act(task.id, action)}>{working === task.id ? busy : label}</button>;
  return <>
    <div className="segments" role="group" aria-label="Show tasks">
      {([["active", "Active", active.length], ["done", "Done", done.length]] as const).map(([id, label, count]) =>
        <button key={id} aria-pressed={showing === id} onClick={() => setShowing(id)}>{label} <span className="count">{count}</span></button>)}
    </div>
    {notice && <p className="settings-note warn" role="status">{notice}</p>}
    {error && <p className="settings-note warn" role="alert">{error}</p>}
    {loading ? <p className="quiet">Loading scheduled tasks…</p> : shown.length === 0 ? <div className="empty">
      <span className="empty-icon"><Icon name="scheduled" /></span>
      {showing === "active" ? <>
        <p className="empty-title">Nothing scheduled</p>
        <p className="empty-desc">End a command with a time to run it later.</p>
        <p className="empty-examples"><kbd>at 9 pm</kbd><kbd>in 3 hours</kbd><kbd>every 3 days</kbd></p>
      </> : <>
        <p className="empty-title">No finished tasks yet</p>
        <p className="empty-desc">Tasks that ran or were stopped show up here.</p>
      </>}
    </div> :
      <ul className="group-rows tasks-list">{shown.map((task) => {
        const [status, tone] = STATUS[task.status];
        return <li className="row task" key={task.id}>
          <span className="row-icon"><Icon name={task.repeat ? "update" : "scheduled"} /></span>
          <div className="row-copy">
            <span className="row-label task-command">{task.command}</span>
            <span className="row-desc">{when(task)}</span>
            {task.summary && <span className="row-desc task-summary">{task.summary}</span>}
            {task.status !== "running" && <span className="task-actions">
              {task.repeat && task.status === "pending" && button(task, "Pause", "Pausing…", api.pauseScheduledTask)}
              {task.status === "paused" && button(task, "Resume", "Resuming…", api.resumeScheduledTask)}
              {(task.status === "pending" || task.status === "paused") && button(task, task.repeat ? "Stop repeating" : "Cancel task", "Stopping…", api.cancelScheduledTask)}
              {task.repeat && (task.status === "cancelled" || task.status === "failed") && button(task, "Move to active", "Moving…", api.resumeScheduledTask)}
            </span>}
          </div>
          <span className={`pill ${tone}`}>{status}</span>
        </li>;
      })}</ul>}
  </>;
}

/** The Settings tab bar, on the title row to the right of "Settings". */
function SettingsTabs({ value, saving, context, onSelect }: { value: Tab; saving: boolean; context: TabContext; onSelect: (tab: Tab) => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [indicator, setIndicator] = useState<{ left: number; width: number } | null>(null);
  const [animate, setAnimate] = useState(false);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    const measure = () => {
      const selected = element.querySelector<HTMLButtonElement>('[aria-selected="true"]');
      if (selected) setIndicator({ left: selected.offsetLeft, width: selected.offsetWidth });
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    const frame = requestAnimationFrame(() => setAnimate(true));
    return () => { observer.disconnect(); cancelAnimationFrame(frame); };
  }, [value]);
  return <div className="tabs" role="tablist" aria-label="Settings sections" ref={ref}>
    {indicator && <span className="tab-highlight" data-animate={animate || undefined} aria-hidden="true" style={indicator} />}
    {TABS.map((tab, index) => {
      const badge = tab.badge(context);
      return <button key={tab.id} role="tab" id={`settings-tab-${tab.id}`}
        aria-selected={value === tab.id} aria-disabled={saving} aria-controls="settings-panel"
        tabIndex={value === tab.id ? 0 : -1} onClick={() => onSelect(tab.id)}
        onKeyDown={(event) => {
          if (saving) return;
          const next = event.key === "ArrowRight" ? (index + 1) % TABS.length
            : event.key === "ArrowLeft" ? (index + TABS.length - 1) % TABS.length
            : event.key === "Home" ? 0 : event.key === "End" ? TABS.length - 1 : null;
          if (next === null) return;
          event.preventDefault();
          onSelect(TABS[next].id);
          ref.current?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[next]?.focus();
        }}>
        <Icon name={tab.icon} />
        <span className="tab-label">{tab.label}</span>
        {badge !== null && <span className="tab-badge" aria-label={`${badge}`}>{badge}</span>}
      </button>;
    })}
  </div>;
}

/** Settings save as they change: no Save button. */
function SettingsView({
  settings,
  onSaved,
  onClose,
  setBusy,
}: {
  settings: Settings;
  onSaved: (s: Settings) => void;
  onClose: () => void;
  setBusy: (b: boolean) => void;
}) {
  const [tab, setTab] = useState<Tab>("general");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [activeTasks, setActiveTasks] = useState<number | null>(null);
  const savePending = useRef(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !savePending.current) {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // The sidebar's count of active scheduled tasks.
  useEffect(() => {
    let alive = true;
    const refresh = () => api.scheduledTasks()
      .then((tasks) => { if (alive) setActiveTasks(tasks.filter((t) => ACTIVE_STATUSES.includes(t.status)).length); })
      .catch(() => {});
    void refresh();
    const timer = window.setInterval(refresh, 4000);
    return () => { alive = false; clearInterval(timer); };
  }, []);

  const persist = async (next: Settings): Promise<boolean> => {
    if (savePending.current) return false;
    savePending.current = true;
    const focused = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setSaving(true);
    try {
      onSaved(await api.saveSettings(next));
      setError(null);
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    } finally {
      savePending.current = false;
      setSaving(false);
      requestAnimationFrame(() => {
        if (focused?.isConnected && document.activeElement === document.body && !focused.matches(":disabled")) focused.focus();
      });
    }
  };

  const context: TabContext = { settings, persist, setBusy, activeTasks };
  const current = TABS.find((t) => t.id === tab)!;

  return (
    <Sheet
      title="Settings"
      aside={<SettingsTabs value={tab} saving={saving} context={context} onSelect={(id) => { if (!savePending.current) setTab(id); }} />}
      bodyClass="settings-body"
      foot={error ? <span className="warn" role="alert">{error}</span> : "Changes save as you go. Esc to go back."}
      footRight={
        <button className="link" aria-disabled={saving} onClick={() => { if (!savePending.current) onClose(); }}>
          Done
        </button>
      }
    >
      <fieldset key={tab} id="settings-panel" role="tabpanel" tabIndex={0} aria-labelledby={`settings-tab-${tab}`} className="tab-panel" disabled={saving} aria-busy={saving}>
        <p className="panel-blurb">{current.blurb}</p>
        {current.render(context)}
      </fieldset>
    </Sheet>
  );
}

type TabProps = { settings: Settings; persist: (s: Settings) => Promise<boolean> };

function GeneralTab({ settings, persist }: TabProps) {
  const [recording, setRecording] = useState(false);
  const [linkError, setLinkError] = useState<string | null>(null);
  return <>
    <Group title="Startup">
      <Row icon="keyboard" label="Open Voxi" description={recording ? "Press the new keys, or Esc to keep the current ones." : "Press this anywhere to open the command box."}>
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
      </Row>
      <Row icon="power" label="Open at login" htmlFor="login" description="Start Voxi in the menu bar when you log in.">
        <input
          id="login"
          className="switch"
          type="checkbox"
          checked={settings.startAtLogin}
          onChange={(e) => persist({ ...settings, startAtLogin: e.target.checked })}
        />
      </Row>
    </Group>
    <Group title="Dictation" foot={linkError ? <span className="warn" role="alert">{linkError}</span> : "Requires Apple silicon and macOS 15 or later. Other dictation apps work too."}>
      <Row icon="mic" label={<>Hex by Anomaly <span className="pill">Recommended</span></>} description="Free, open source dictation that runs on your Mac.">
        <button className="secondary small" onClick={() => { setLinkError(null); api.openHex().catch((e) => setLinkError(String(e))); }}>Get Hex</button>
      </Row>
    </Group>
    <Group title="Updates">
      <UpdatesRow />
    </Group>
    <Group title="Privacy">
      <ReportsRow />
    </Group>
  </>;
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
    <Row icon="update" className="updates" label={<>Voxi {version} {state.k === "latest" && <span className="pill ok">Up to date</span>}</>}
      description={status ? <span className={state.k === "error" ? "warn" : undefined} role="status">{status}</span> : "Your settings carry over."}>
      <button className="secondary small" disabled={busy} onClick={() => checkAndInstall(setState)}>
        Check for updates
      </button>
      {(state.k === "downloading" || state.k === "restarting" || notes.length > 0) && (
        <div className="update-status">
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
    </Row>
  );
}

/** Wrong-result reports kept on this Mac, with ways to see or clear them. */
function ReportsRow() {
  const [reports, setReports] = useState<Reports | null>(null);
  const [note, setNote] = useState<string | null>(null);

  useEffect(() => {
    api.reportsSummary().then(setReports).catch(() => setReports(null));
  }, []);

  const reportAction = async (run: () => Promise<Reports | void>) => {
    try {
      const r = await run();
      if (r) setReports(r);
      setNote(null);
    } catch (e) {
      setNote(String(e));
    }
  };

  const count = reports?.count ?? 0;
  return (
    <Row icon="shield" label="Wrong-result reports"
      description={note ? <span className="warn" role="alert">{note}</span>
        : `${count === 0 ? "None" : count} saved. Nothing leaves this Mac.`}>
      <button className="secondary small" disabled={!count} onClick={() => reportAction(api.revealReports)}>Show in Finder</button>
      <button className="link remove" disabled={!count} onClick={() => reportAction(api.clearReports)}>Clear</button>
    </Row>
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
  const add = () => {
    setList((l) => [...l, { phrase: "", commands: [] }]);
    setEditing(list.length);
  };
  const foot = "When Voxi misses a phrase and you then run a command that works, it offers to learn the phrase. Learned phrases appear here too.";

  if (list.length === 0) {
    return <>
      <div className="empty">
        <span className="empty-icon"><Icon name="shortcuts" /></span>
        <p className="empty-title">No shortcuts yet</p>
        <p className="empty-desc">Say “run my usuals” to move today's screenshots and clear old installers, all in one preview.</p>
        <button className="secondary small" onClick={add}>Add shortcut</button>
      </div>
      <p className="group-foot">{foot}</p>
    </>;
  }

  return (
    <Group title="Saved shortcuts" action={editing === null && <button className="link strong" onClick={add}>+ Add</button>} foot={foot}>
      {list.map((s, i) =>
        editing === i ? (
          <li className="row shortcut" key={i}>
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
              writingsuggestions="false"
              spellCheck={false}
              autoCorrect="off"
              className="plain commands"
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
          <li className="row clickable" key={i}>
            <button className="shortcut-row" onClick={() => setEditing(i)}>
              <span className="row-icon"><Icon name="shortcuts" /></span>
              <span className="row-copy">
                <span className="row-label">“{s.phrase || "Untitled"}”</span>
                <span className="row-desc">{s.commands.filter((c) => c.trim()).slice(0, 2).join(" · ") || "No commands yet"}</span>
              </span>
              <span className="pill">{count(s)} {count(s) === 1 ? "command" : "commands"}</span>
              <span className="chevron" aria-hidden="true">›</span>
            </button>
          </li>
        ),
      )}
    </Group>
  );
}

/** "~/Work" from "/Users/me/Work/", the form allowed_folders returns. */
const shownFolder = (path: string) => path.replace(/^\/Users\/[^/]+/, "~").replace(/\/$/, "");

/** The four standard folders are always allowed; more can be added. Below
 *  them, the nicknames Voxi knows folders by. */
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

  return <>
    <Group title="Allowed folders" foot={note ? <span className="warn" role="status">{note}</span> : "Voxi only changes files inside these folders."}>
      <Row className="add-row">
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
        {adding.trim()
          ? <button className="secondary small" onClick={() => add(adding.trim())}>Add</button>
          : <button className="secondary small" onClick={choose}>Choose…</button>}
      </Row>
      {builtIn.map((f) => (
        <Row key={f} className="compact" icon="folders" label={<span className="pref-folder">{f}</span>}>
          <span className="pill">Built-in</span>
        </Row>
      ))}
      {extra.map((f) => (
        <Row key={f} className="compact" icon="folders" label={<span className="pref-folder">{tildify(f)}</span>}>
          <span className="pill ok">Added</span>
          <button className="link remove" onClick={() => persist({ ...settings, extraFolders: extra.filter((x) => x !== f) })}>
            Remove
          </button>
        </Row>
      ))}
    </Group>
    <NicknamesGroup settings={settings} persist={persist} setBusy={setBusy} />
  </>;
}

/** Your names for folders, so “move it to dev” finds the right one. */
function NicknamesGroup({ settings, persist, setBusy }: TabProps & { setBusy: (b: boolean) => void }) {
  const [name, setName] = useState("");
  const [note, setNote] = useState<string | null>(null);

  // The picker takes focus, which would otherwise close the window.
  const choose = async () => {
    const said = name.trim();
    if (!said) return setNote("Type the name you'll say first, like “dev”.");
    setBusy(true);
    try {
      const path = await api.pickFolder();
      if (path && await persist({ ...settings, nicknames: [...settings.nicknames, { name: said, path }] })) {
        setName("");
        setNote(null);
      }
    } catch (e) {
      setNote(`Couldn't open the folder picker: ${e}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Group title="Nicknames" foot={note ? <span className="warn" role="status">{note}</span> : "Say “move it to dev” and Voxi knows where. It also offers to remember a folder after you pick one."}>
      <Row className="add-row">
        <input
          className="path"
          value={name}
          placeholder="Name to say, like dev"
          aria-label="Nickname to say"
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") { e.preventDefault(); choose(); }
          }}
        />
        <button className="secondary small" onClick={choose}>Choose folder…</button>
      </Row>
      {settings.nicknames.map((n) => (
        <Row key={n.name} className="compact" icon="tag" label={`“${n.name}”`} description={tildify(n.path)}>
          <button className="link remove" onClick={() => persist({ ...settings, nicknames: settings.nicknames.filter((x) => x.name !== n.name) })}>
            Remove
          </button>
        </Row>
      ))}
    </Group>
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
